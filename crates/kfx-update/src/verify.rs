//! Integrity of downloads: `SHA256SUMS.txt` parsing, SHA-256 of files, and the [`Verifier`]
//! hook where a signature check can be added.
//!
//! # Trust model
//!
//! An update is installed only when its SHA-256 matches the line for it in the `SHA256SUMS.txt`
//! of the same GitHub release, both fetched over HTTPS from GitHub's hosts. This protects
//! against corrupted and truncated downloads and against a tampered mirror or proxy, but not
//! against someone who controls the GitHub account or repository: they could publish matching
//! files. Releases are not signed with an offline key.
//!
//! A signature check would slot in as a [`Verifier`]: the maintainer signs `SHA256SUMS.txt` with
//! a key kept offline (for example with minisign, publishing `SHA256SUMS.txt.minisig` as a release
//! asset), the public key is compiled into the editor, and the verifier refuses a sums file whose
//! signature does not check out. Because every asset is checked against the sums file, signing
//! that one file covers every download.

use crate::release::{Asset, Release};
use crate::{Error, Result};
use sha2::Digest;
use std::io::Read;
use std::path::Path;

/// The checksums of a release (`SHA256SUMS.txt`, as written by `sha256sum`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sums {
    entries: Vec<(String, String)>,
}

impl Sums {
    /// Parse `<64 hex digits> <space> <space or *> <file name>` lines (the format of
    /// `sha256sum`, text or binary mode). Blank lines and `#` comments are skipped; any other
    /// malformed line, or one file listed twice with different sums, is an error.
    pub fn parse(text: &str) -> Result<Sums> {
        let mut entries: Vec<(String, String)> = vec![];
        for (i, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || Error::parse("SHA256SUMS.txt", format!("line {}: {line:?}", i + 1));
            let (hash, rest) = line.split_once(' ').ok_or_else(bad)?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad());
            }
            let name = rest
                .strip_prefix(' ')
                .or_else(|| rest.strip_prefix('*'))
                .ok_or_else(bad)?;
            if name.is_empty() || name.contains(['/', '\\']) {
                return Err(bad());
            }
            let hash = hash.to_ascii_lowercase();
            match entries.iter().find(|(n, _)| n == name) {
                Some((_, h)) if *h != hash => {
                    return Err(Error::parse(
                        "SHA256SUMS.txt",
                        format!("{name} is listed twice with different sums"),
                    ))
                }
                Some(_) => {}
                None => entries.push((name.to_string(), hash)),
            }
        }
        Ok(Sums { entries })
    }

    /// The lower-case hex SHA-256 of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Lower-case hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn sha256_bytes(b: &[u8]) -> String {
    hex(&sha2::Sha256::digest(b))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Check a downloaded file against the release's sums: size first, then SHA-256.
pub fn check_file(sums: &Sums, asset: &Asset, file: &Path) -> Result<String> {
    let size = std::fs::metadata(file)
        .map_err(|e| Error::io(file, e))?
        .len();
    if size != asset.size {
        return Err(Error::SizeMismatch {
            name: asset.name.clone(),
            expected: asset.size,
            actual: size,
        });
    }
    let expected = sums
        .get(&asset.name)
        .ok_or_else(|| Error::MissingChecksum(asset.name.clone()))?;
    let actual = sha256_file(file)?;
    if actual != expected {
        return Err(Error::ChecksumMismatch {
            name: asset.name.clone(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(actual)
}

/// A check of the release's `SHA256SUMS.txt` before it is trusted. The sums file is what
/// vouches for every download, so this is where a signature check belongs.
pub trait Verifier: Send + Sync {
    /// Accept or refuse `sums` (the raw bytes of `SHA256SUMS.txt`) of `release`. `fetch` fetches
    /// another (small) asset of the same release, such as a detached signature.
    fn verify_sums(
        &self,
        release: &Release,
        sums: &[u8],
        fetch: &mut dyn FnMut(&Asset) -> Result<Vec<u8>>,
    ) -> Result<()>;

    /// A short description for the logs and the About dialog.
    fn describe(&self) -> &'static str;
}

/// The verifier in use today: the sums file is trusted as served by GitHub over HTTPS (see the
/// module documentation for what that means).
pub struct HttpsOnly;

impl Verifier for HttpsOnly {
    fn verify_sums(
        &self,
        _release: &Release,
        _sums: &[u8],
        _fetch: &mut dyn FnMut(&Asset) -> Result<Vec<u8>>,
    ) -> Result<()> {
        Ok(())
    }
    fn describe(&self) -> &'static str {
        "SHA-256 from the release's SHA256SUMS.txt over HTTPS (not signed)"
    }
}
