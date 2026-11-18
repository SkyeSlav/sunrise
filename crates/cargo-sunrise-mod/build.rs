//! The commit the tool is built from, for `--version`: `SUNRISE_TOOL_COMMIT` when the build names one (the release
//! workflows do), else the checkout's `HEAD`, else `unknown` (a source archive has no history).
fn main() {
    println!("cargo:rerun-if-env-changed=SUNRISE_TOOL_COMMIT");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let commit = std::env::var("SUNRISE_TOOL_COMMIT")
        .ok()
        .filter(|commit| !commit.is_empty())
        .or_else(|| {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SUNRISE_TOOL_BUILT_FROM={commit}");
}
