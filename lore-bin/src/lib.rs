use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    WindowsX86_64,
    LinuxX86_64,
    LinuxAarch64,
    MacOsAarch64,
}

/// Downloads the binary from github
pub fn fetch_binary(target: Target) -> PathBuf {
    let (prefix, triple, ext) = match target {
        Target::WindowsX86_64 => ("lore", "x86_64-pc-windows-msvc", "dll"),
        Target::LinuxX86_64 => ("liblore", "x86_64-unknown-linux-gnu", "so"),
        Target::LinuxAarch64 => ("liblore", "aarch64-unknown-linux-gnu", "so"),
        Target::MacOsAarch64 => ("liblore", "aarch64-apple-darwin", "dylib"),
    };

    const LORE_VERSION_TAG: &str = "v0.8.4-nightly-348e940";
    const LORE_RELEASE_BASE_URL: &str =
        "https://github.com/Traverse-Research/Lore-rust-bindings/releases/download";

    let url = format!(
        "{LORE_RELEASE_BASE_URL}/{LORE_VERSION_TAG}/{prefix}-{LORE_VERSION_TAG}-{triple}.{ext}"
    );

    let out_dir =
        std::env::var_os("OUT_DIR").expect("OUT_DIR not set (are you in a build script?)");
    let path = PathBuf::from(out_dir).join(format!("{prefix}.{ext}"));

    let status = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(&path)
        .arg(&url)
        .status()
        .unwrap_or_else(|e| panic!("failed to run curl: {e}"));
    assert!(status.success(), "curl failed to download {url}");

    path
}
