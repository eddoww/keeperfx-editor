//! The running platform and the release asset that belongs to it.
//!
//! Asset names are made by `.github/workflows/release.yml`: `kfx-editor-<tag>-<platform>.<ext>`.

use crate::release::{Asset, Release};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    MacOs,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

/// Which package of a release to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    /// The archive of the release (`.tar.gz`, or `.zip` on Windows): unpacked and swapped in.
    Archive,
    /// The Windows installer (`-setup.exe`), run silently for installer-based installs.
    WindowsSetup,
}

impl Platform {
    pub fn current() -> Platform {
        Platform::from_names(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn from_names(os: &str, arch: &str) -> Platform {
        Platform {
            os: match os {
                "linux" => Os::Linux,
                "windows" => Os::Windows,
                "macos" => Os::MacOs,
                _ => Os::Other,
            },
            arch: match arch {
                "x86_64" => Arch::X86_64,
                "aarch64" => Arch::Aarch64,
                _ => Arch::Other,
            },
        }
    }

    /// The platform part of the asset names (`linux-x86_64`), when releases are built for it.
    pub fn release_name(self) -> Option<&'static str> {
        Some(match (self.os, self.arch) {
            (Os::Linux, Arch::X86_64) => "linux-x86_64",
            (Os::Windows, Arch::X86_64) => "windows-x86_64",
            (Os::MacOs, Arch::Aarch64) => "macos-arm64",
            (Os::MacOs, Arch::X86_64) => "macos-x86_64",
            _ => return None,
        })
    }

    /// The end of the asset name of `kind` (`-linux-x86_64.tar.gz`).
    pub fn asset_suffix(self, kind: PackageKind) -> Option<String> {
        let name = self.release_name()?;
        Some(match (self.os, kind) {
            (Os::Windows, PackageKind::Archive) => format!("-{name}.zip"),
            (Os::Windows, PackageKind::WindowsSetup) => format!("-{name}-setup.exe"),
            (_, PackageKind::WindowsSetup) => return None,
            _ => format!("-{name}.tar.gz"),
        })
    }

    /// The executable's file name.
    pub fn exe_name(self) -> &'static str {
        if self.os == Os::Windows {
            "kfx-editor.exe"
        } else {
            "kfx-editor"
        }
    }

    pub fn describe(self) -> String {
        self.release_name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
    }
}

/// The asset of `kind` for `platform`: `kfx-editor-<tag><suffix>`, or failing that the single
/// asset named `kfx-editor-...<suffix>` (a tag written differently in the file name).
pub fn select_asset(release: &Release, platform: Platform, kind: PackageKind) -> Option<&Asset> {
    let suffix = platform.asset_suffix(kind)?;
    let exact = format!("kfx-editor-{}{suffix}", release.tag_name);
    if let Some(a) = release.asset(&exact) {
        return Some(a);
    }
    let mut it = release.assets.iter().filter(|a| {
        a.name.starts_with("kfx-editor-")
            && a.name.ends_with(&suffix)
            // `-windows-x86_64.zip` must not match a `...-setup.exe`, and the reverse
            && !a.name[..a.name.len() - suffix.len()].ends_with("-setup")
    });
    let one = it.next()?;
    it.next().is_none().then_some(one)
}

/// The checksum file of a release.
pub const SUMS_NAME: &str = "SHA256SUMS.txt";
