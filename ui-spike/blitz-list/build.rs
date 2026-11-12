fn main() {
    // Two renderers on would let the label name the one that did not draw: vello-cpu needs --no-default-features.
    assert!(
        std::env::var_os("CARGO_FEATURE_VELLO_CPU").is_none()
            || std::env::var_os("CARGO_FEATURE_VELLO_HYBRID").is_none(),
        "pick one renderer: --no-default-features --features window,vello-cpu"
    );
    let renderer = if std::env::var_os("CARGO_FEATURE_VELLO_CPU").is_some() {
        "vello-cpu"
    } else {
        "vello-hybrid"
    };
    println!("cargo:rustc-env=BLITZ_RENDERER={renderer}");
}
