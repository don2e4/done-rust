//! Embeds the Windows manifest into this package's examples. A library cannot
//! embed resources into another package's executable; applications add the
//! same two linker arguments to their own build script (see README).
fn main() {
    println!("cargo::rerun-if-changed=windows.manifest");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.ends_with("-windows-msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows.manifest");
        println!("cargo::rustc-link-arg-examples=/MANIFEST:EMBED");
        println!(
            "cargo::rustc-link-arg-examples=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
