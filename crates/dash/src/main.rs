//! `dash serve --data <dir>`: one role of the one binary (section 3), on loopback. It prints `port <n>`, then answers
//! GET /ping, GET /memory and GET /scan. Plain std::net until the surface crate exists.
use dash_core::{Store, ping};
use dash_meter::memory::memory;
use dash_store as store;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget: the whole request
/// has this long, however its bytes trickle in.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const REQUEST_LIMIT: u64 = 64 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", "--data", dir] => serve(PathBuf::from(dir)),
        _ => {
            eprintln!("usage: dash serve --data <dir>");
            std::process::exit(2);
        }
    }
}

fn serve(dir: PathBuf) {
    let db = store::open(&dir).expect("open the store");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    println!("port {}", listener.local_addr().expect("address").port());
    std::io::stdout().flush().expect("stdout");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let answer = request_line(&stream, REQUEST_DEADLINE).map(|line| route(&line, &db));
        let (status, body) = answer.unwrap_or_else(|| ("400 Bad Request", "bad request".into()));
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

/// A stream whose reads all end by one deadline: a timeout per read restarts at each byte.
struct ByDeadline<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for ByDeadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        (&*self.stream).read(buf)
    }
}

/// The request's first line, its headers read to the blank line: closing on unread bytes sends a reset that can destroy
/// the answer.
fn request_line(stream: &TcpStream, limit: Duration) -> Option<String> {
    let timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut reader = BufReader::new(timed.take(REQUEST_LIMIT));
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header).ok()?;
        if n == 0 || header == "\r\n" || header == "\n" {
            return Some(first);
        }
    }
}

fn route(line: &str, db: &dyn Store) -> (&'static str, String) {
    let mut words = line.split_whitespace();
    match (words.next(), words.next()) {
        (Some("GET"), Some("/ping")) => ("200 OK", ping().into()),
        (Some("GET"), Some("/memory")) => {
            let m = memory();
            ("200 OK", format!("{:.1} {:.1}", m.current_mb, m.peak_mb))
        }
        (Some("GET"), Some("/scan")) => match db.scan() {
            Ok(s) => ("200 OK", format!("{} {}", s.rows, s.bytes)),
            Err(e) => ("500 Internal Server Error", e),
        },
        _ => ("404 Not Found", "no such path".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::net::TcpListener;

    #[test]
    fn a_request_trickling_in_ends_at_its_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        std::thread::spawn(move || {
            // each byte well inside what a timeout per read would allow
            while client.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        assert_eq!(request_line(&server, Duration::from_millis(200)), None);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
