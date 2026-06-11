fn main() {
    // Capture git commit hash at build time for um_build_info metric.
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=BUILD_COMMIT={}", commit);
    println!("cargo:rerun-if-changed=.git/HEAD");
}
