//! The numbers of section 20's rust-registry-ext row: `cargo run --release -p dash --example extension_costs`. Boot and
//! footprint of Steel, then one extractor call through flat dispatch for each of the three kinds.
use dash_core::registry::Registry;
use dash_ext as ext;
use dash_ingest as _;
use dash_meter::memory::memory;
use std::path::Path;
use std::time::Instant;

const CALLS: [(&str, u32); 3] = [
    ("text/plain", 100_000),
    ("text/x-shout", 10_000),
    ("text/x-rot13", 200),
];

fn main() {
    let dir = std::env::temp_dir().join(format!("dash-extension-costs-{}", std::process::id()));
    let folder = dir.join(ext::FOLDER);
    std::fs::create_dir_all(&folder).expect("folder");
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ext/extensions.d");
    for entry in std::fs::read_dir(samples).expect("samples").flatten() {
        std::fs::copy(entry.path(), folder.join(entry.file_name())).expect("copy");
    }

    std::fs::write(folder.join("30-rot13.json"), ext::pod::sample_manifest()).expect("the pod");

    let start = Instant::now();
    let base = ext::script::base_engine();
    println!(
        "first engine: {:?}, footprint {:.1} MB",
        start.elapsed(),
        memory().current_mb
    );
    let start = Instant::now();
    let another = ext::script::base_engine();
    println!(
        "another engine: {:?}, footprint {:.1} MB",
        start.elapsed(),
        memory().current_mb
    );
    drop(another);
    let before = memory().current_mb;
    let start = Instant::now();
    let clones: Vec<_> = (0..100).map(|_| base.clone()).collect();
    println!(
        "100 clones: {:?}, +{:.2} MB",
        start.elapsed(),
        memory().current_mb - before
    );
    drop(clones);
    drop(base);

    let start = Instant::now();
    let mut registry = Registry::with_compiled().expect("compiled");
    let refused = ext::load(&mut registry, &dir, |_| Ok(String::new()));
    assert!(refused.is_empty(), "{refused:?}");
    println!(
        "start with the samples: {:?}, footprint {:.1} MB, {} keys",
        start.elapsed(),
        memory().current_mb,
        registry.listing().len()
    );

    for (key, calls) in CALLS {
        let start = Instant::now();
        registry.extract(key, b"hello").expect("first call");
        let first = start.elapsed();
        let start = Instant::now();
        for _ in 0..calls {
            registry.extract(key, b"hello").expect("call");
        }
        println!(
            "{key}: first call {first:?}, then {:?} a call over {calls}",
            start.elapsed() / calls
        );
    }
    println!("peak {:.1} MB", memory().peak_mb);
    drop(registry);
    let _ = std::fs::remove_dir_all(&dir);
}
