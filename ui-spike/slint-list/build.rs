use std::path::Path;

fn main() {
    slint_build::compile("ui/app.slint").expect("slint compile");
    // Checked, never included.
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let deps = slint_build::compile_with_output_path(
        "ui/all-components.slint",
        Path::new(&out).join("all_components.rs"),
        slint_build::CompilerConfiguration::new(),
    )
    .expect("slintcn, every component");
    // compile() narrows cargo's watch to its own imports, so these need naming.
    for dep in deps {
        println!("cargo:rerun-if-changed={}", dep.display());
    }
}
