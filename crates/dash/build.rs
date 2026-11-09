//! The /deps page, generated from the code at each build.
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    let root = Path::new(&manifest).join("../..");
    dash_gate::graph::write_page(&root, &Path::new(&out).join("deps.txt"));
}
