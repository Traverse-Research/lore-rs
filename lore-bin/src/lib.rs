use std::path::PathBuf;

/// The Lore release the bindings are generated for, and so the one whose
/// binaries to download. Must match the tag the `lore` submodule is pinned to,
/// CI verifies this.
pub const LORE_VERSION: &str = "0.10.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    WindowsX86_64,
    LinuxX86_64,
    LinuxAarch64,
    MacOsAarch64,
}

impl Target {
    /// Returns the target cargo is compiling for, from the environment
    /// variables that cargo sets for build scripts, or [`None`] if Lore ships
    /// no binaries for it.
    pub fn from_build_env() -> Option<Self> {
        let os = std::env::var("CARGO_CFG_TARGET_OS")
            .expect("CARGO_CFG_TARGET_OS not set (are you in a build script?)");
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH")
            .expect("CARGO_CFG_TARGET_ARCH not set (are you in a build script?)");
        match (os.as_str(), arch.as_str()) {
            ("windows", "x86_64") => Some(Self::WindowsX86_64),
            ("linux", "x86_64") => Some(Self::LinuxX86_64),
            ("linux", "aarch64") => Some(Self::LinuxAarch64),
            ("macos", "aarch64") => Some(Self::MacOsAarch64),
            _ => None,
        }
    }
}

/// The file name of the Lore dynamic library on the given target.
pub fn library_file_name(target: Target) -> &'static str {
    match target {
        Target::WindowsX86_64 => "lore.dll",
        Target::LinuxX86_64 | Target::LinuxAarch64 => "liblore.so",
        Target::MacOsAarch64 => "liblore.dylib",
    }
}

/// The name of the Lore release asset holding the library for the given
/// target, as the Lore GitHub release names it: a zip on Windows, a tar.gz
/// elsewhere. A mirror of the release keeps the same names.
pub fn archive_name(target: Target) -> String {
    let (triple, archive_ext) = match target {
        Target::WindowsX86_64 => ("x86_64-pc-windows-msvc", "zip"),
        Target::LinuxX86_64 => ("x86_64-unknown-linux-gnu", "tar.gz"),
        Target::LinuxAarch64 => ("aarch64-unknown-linux-gnu-neoverse-512tvb", "tar.gz"),
        Target::MacOsAarch64 => ("aarch64-apple-darwin", "tar.gz"),
    };
    format!("liblore-v{LORE_VERSION}-{triple}.{archive_ext}")
}

/// Where Epic publishes the release assets: `v<version>/<asset>` under here.
const GITHUB_RELEASES: &str = "https://github.com/EpicGames/lore/releases/download";

/// Downloads the prebuilt Lore library archive from the Lore GitHub release
/// matching the `lore` submodule, extracts it into `OUT_DIR` and returns the
/// path to the contained library.
pub fn fetch_binary(target: Target) -> PathBuf {
    fetch_binary_from(GITHUB_RELEASES, target)
}

/// As [`fetch_binary`], but from a mirror of the release: `mirror` is the URL
/// under which the assets are found as `v<version>/<asset>`, named as the
/// GitHub release names them. For a build that should depend on storage of
/// its own rather than on GitHub's release hosts.
pub fn fetch_binary_from(mirror: &str, target: Target) -> PathBuf {
    let archive_name = archive_name(target);
    let url = format!("{mirror}/v{LORE_VERSION}/{archive_name}");

    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").expect("OUT_DIR not set (are you in a build script?)"),
    );
    let archive = out_dir.join(&archive_name);

    let status = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            // Cargo shows a build script's stderr only when it fails, so this
            // trace (addresses tried, OS error per attempt) costs nothing on a
            // green build and explains the red ones.
            "--verbose",
            "--retry",
            "4",
            // `--retry` alone skips connection failures (curl exit 7), so a
            // mirror that is briefly unreachable fails the build on the first try.
            "--retry-all-errors",
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

    // Windows ships bsdtar in System32, which extracts both zip and tar.gz.
    // Resolve it explicitly: PATH may find GNU tar first (e.g. Git Bash),
    // which cannot read zip. GNU tar on other hosts cannot either, so when
    // cross-compiling to Windows from Linux/macOS extract the zip with unzip.
    let mut extract = if cfg!(windows) {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let mut cmd =
            std::process::Command::new(PathBuf::from(system_root).join("System32\\tar.exe"));
        cmd.arg("-xf").arg(&archive).arg("-C").arg(&out_dir);
        cmd
    } else if archive_name.ends_with(".zip") {
        let mut cmd = std::process::Command::new("unzip");
        cmd.arg("-o").arg(&archive).arg("-d").arg(&out_dir);
        cmd
    } else {
        let mut cmd = std::process::Command::new("tar");
        cmd.arg("-xf").arg(&archive).arg("-C").arg(&out_dir);
        cmd
    };

    let status = extract
        .status()
        .unwrap_or_else(|e| panic!("failed to run {:?}: {e}", extract.get_program()));

    assert!(
        status.success(),
        "{:?} failed to extract {}",
        extract.get_program(),
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
