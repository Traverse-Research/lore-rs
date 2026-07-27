use std::path::PathBuf;

/// The Lore release to download binaries from. Must match the tag the `lore`
/// submodule is pinned to, CI verifies this.
const LORE_VERSION: &str = "0.8.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    WindowsX86_64,
    LinuxX86_64,
    LinuxAarch64,
    MacOsAarch64,
}

impl Target {
    /// Returns the target cargo is compiling for, from the environment
    /// variables that cargo sets for build scripts.
    pub fn from_build_env() -> Self {
        let os = std::env::var("CARGO_CFG_TARGET_OS")
            .expect("CARGO_CFG_TARGET_OS not set (are you in a build script?)");
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH")
            .expect("CARGO_CFG_TARGET_ARCH not set (are you in a build script?)");
        match (os.as_str(), arch.as_str()) {
            ("windows", "x86_64") => Self::WindowsX86_64,
            ("linux", "x86_64") => Self::LinuxX86_64,
            ("linux", "aarch64") => Self::LinuxAarch64,
            ("macos", "aarch64") => Self::MacOsAarch64,
            (os, arch) => panic!("no Lore binaries for {os}/{arch}"),
        }
    }
}

/// The file name of the Lore dynamic library on the given target, as the OS
/// loader expects it (e.g. what to name the copy placed next to your
/// executable).
pub fn library_file_name(target: Target) -> &'static str {
    match target {
        Target::WindowsX86_64 => "lore.dll",
        Target::LinuxX86_64 | Target::LinuxAarch64 => "liblore.so",
        Target::MacOsAarch64 => "liblore.dylib",
    }
}

/// Downloads the prebuilt Lore library archive from the Lore GitHub release
/// matching the `lore` submodule, extracts it into `OUT_DIR` and returns the
/// path to the contained library.
pub fn fetch_binary(target: Target) -> PathBuf {
    let (triple, archive_ext) = match target {
        Target::WindowsX86_64 => ("x86_64-pc-windows-msvc", "zip"),
        Target::LinuxX86_64 => ("x86_64-unknown-linux-gnu", "tar.gz"),
        Target::LinuxAarch64 => ("aarch64-unknown-linux-gnu-neoverse-512tvb", "tar.gz"),
        Target::MacOsAarch64 => ("aarch64-apple-darwin", "tar.gz"),
    };

    let url = format!(
        "https://github.com/EpicGames/lore/releases/download/v{LORE_VERSION}/liblore-v{LORE_VERSION}-{triple}.{archive_ext}"
    );

    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").expect("OUT_DIR not set (are you in a build script?)"),
    );
    let archive = out_dir.join(format!("liblore.{archive_ext}"));

    let status = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--max-time",
            "300",
            "--output",
        ])
        .arg(&archive)
        .arg(&url)
        .status()
        .unwrap_or_else(|e| panic!("failed to run curl: {e}"));
    assert!(status.success(), "curl failed to download {url}");

    // Windows ships bsdtar in System32, which also extracts zip. Resolve it
    // explicitly: PATH may find GNU tar first (e.g. Git Bash), which cannot.
    let tar = if cfg!(windows) {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(system_root).join("System32\\tar.exe")
    } else {
        PathBuf::from("tar")
    };

    let status = std::process::Command::new(&tar)
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&out_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", tar.display()));

    assert!(
        status.success(),
        "tar failed to extract {}",
        archive.display()
    );

    out_dir.join(library_file_name(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "downloads the Lore release from the network"]
    fn fetch_binary_for_host() {
        let target = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Target::WindowsX86_64
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Target::LinuxX86_64
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            Target::LinuxAarch64
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Target::MacOsAarch64
        } else {
            panic!("no Lore binaries for this host");
        };

        let out_dir = std::env::temp_dir().join("lore-bin-fetch-test");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::env::set_var("OUT_DIR", &out_dir);

        let library = fetch_binary(target);
        assert!(library.exists());
    }
}
