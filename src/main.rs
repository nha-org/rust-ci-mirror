//! `dash serve --data <dir>`: one role of the one binary (section 3), on loopback. It prints `port <n>`, then answers
//! GET /ping, GET /memory and GET /scan. Plain std::net until the surface crate exists.
use dash::{memory::memory, ping, store};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
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
        let answer = request_line(&stream).map(|line| route(&line, &db));
        let (status, body) = answer.unwrap_or_else(|| ("400 Bad Request", "bad request".into()));
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

/// The request's first line, its headers read to the blank line: closing on unread bytes sends a reset that can destroy
/// the answer.
fn request_line(stream: &TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream.take(REQUEST_LIMIT));
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

fn route(line: &str, db: &rusqlite::Connection) -> (&'static str, String) {
    let mut words = line.split_whitespace();
    match (words.next(), words.next()) {
        (Some("GET"), Some("/ping")) => ("200 OK", ping().into()),
        (Some("GET"), Some("/memory")) => {
            let m = memory();
            ("200 OK", format!("{:.1} {:.1}", m.current_mb, m.peak_mb))
        }
        (Some("GET"), Some("/scan")) => match store::scan(db) {
            Ok(s) => ("200 OK", format!("{} {}", s.rows, s.bytes)),
            Err(e) => ("500 Internal Server Error", e.to_string()),
        },
        _ => ("404 Not Found", "no such path".into()),
    }
}
