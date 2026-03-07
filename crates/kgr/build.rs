fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-env=KGR_SOURCE_DIR={manifest_dir}");
    println!("cargo:rerun-if-changed=build.rs");
}
