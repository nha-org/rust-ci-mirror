//! The parity run's Rust side (task rust-forwarded-next): `cargo run -p dash-store --example older_text -- <older db> store
//! text <source>` prints what the front door answers from Rust for it, its exit code last on stderr.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((db, words)) = args.split_first() else {
        eprintln!("usage: older_text <older db> store text|files <words>");
        std::process::exit(2);
    };
    match dash_store::older_text::answer(std::path::Path::new(db), words) {
        Ok(Some((code, out, err))) => {
            print!("{out}");
            eprint!("{err}");
            std::process::exit(code);
        }
        Ok(None) => {
            eprintln!("the older app answers this one");
            std::process::exit(98);
        }
        Err(e) => {
            eprintln!("the reader failed: {e}");
            std::process::exit(99);
        }
    }
}
