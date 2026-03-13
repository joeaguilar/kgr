fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-env=KGR_SOURCE_DIR={manifest_dir}");

    // Workspace root is two levels up from crates/kgr/
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new(&manifest_dir));
    let git_dir = workspace_root.join(".git");

    // Git-based version: uses `git describe --tags --always --dirty`
    // Falls back to CARGO_PKG_VERSION if git is unavailable.
    let version = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(
            || std::env::var("CARGO_PKG_VERSION").unwrap(),
            |s| s.trim().to_string(),
        );

    println!("cargo:rustc-env=KGR_VERSION={version}");
    // Re-run on any git state change (new commits, tags, branch switch)
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("refs/tags").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("refs/heads").display()
    );
}
