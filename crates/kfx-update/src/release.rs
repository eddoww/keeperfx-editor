//! GitHub releases: the JSON model, versions, channels and the choice of the update.

use semver::Version;
use serde::{Deserialize, Serialize};

/// A release as listed by `GET /repos/{owner}/{repo}/releases` (only the fields used).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The release notes (Markdown written by the maintainer; untrusted text for the app).
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    /// `sha256:<hex>` as computed by GitHub on upload (newer releases); informative only, the
    /// checksum that counts is the one in `SHA256SUMS.txt`.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Release {
    /// The version of the tag, if it is one (see [`parse_version`]).
    pub fn version(&self) -> Option<Version> {
        parse_version(&self.tag_name)
    }
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Parse the JSON list of releases.
pub fn parse_releases(json: &[u8]) -> crate::Result<Vec<Release>> {
    serde_json::from_slice(json)
        .map_err(|e| crate::Error::parse("the list of releases", e.to_string()))
}

/// A version from a tag: `v1.2.3`, `1.2.3`, `V1.2.3-rc.1`. A two-part tag (`v1.2`) is read as
/// `1.2.0`. Anything else (`nightly`, `1.2.3.4`, `v1.x`) is not a version.
pub fn parse_version(tag: &str) -> Option<Version> {
    let t = tag.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    if let Ok(v) = Version::parse(t) {
        return Some(v);
    }
    // 1.2 -> 1.2.0 (only plain numbers)
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() == 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Version::parse(&format!("{t}.0")).ok();
    }
    None
}

/// The version of this build is a development version: a pre-release or build suffix
/// (`0.2.0-dev`, `0.2.0+dirty`).
pub fn is_dev_version(v: &str) -> bool {
    parse_version(v).is_none_or(|v| !v.pre.is_empty() || !v.build.is_empty())
}

/// Which releases count as updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    /// Only releases not marked as pre-releases on GitHub.
    #[serde(rename = "stable")]
    Stable,
    /// Every published release, pre-releases included. The default: every release of the
    /// editor so far is marked as a pre-release, so the stable channel would never offer one.
    #[default]
    #[serde(rename = "pre-release")]
    PreRelease,
}

impl Channel {
    pub fn includes(self, r: &Release) -> bool {
        !r.draft && (self == Channel::PreRelease || !r.prerelease)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::PreRelease => "pre-release",
        }
    }
}

/// The release to offer: the highest version (by semver precedence) of the channel that is
/// newer than `current`. Drafts and tags that are not versions are ignored. A version-1.2.3-rc
/// tag counts as a pre-release by semver even when GitHub does not mark it so, and is offered on
/// the stable channel only if GitHub marks it stable.
///
/// When the user skipped that highest version, nothing is offered (older releases are
/// superseded by the skipped one); a release newer than the skipped one is offered again.
pub fn select<'a>(
    releases: &'a [Release],
    current: &Version,
    channel: Channel,
    skipped: &[String],
) -> Option<(&'a Release, Version)> {
    let best = releases
        .iter()
        .filter(|r| channel.includes(r))
        .filter_map(|r| r.version().map(|v| (r, v)))
        .filter(|(_, v)| v > current)
        .max_by(|a, b| a.1.cmp(&b.1))?;
    let skipped_it = skipped
        .iter()
        .filter_map(|s| parse_version(s))
        .any(|s| s == best.1);
    (!skipped_it).then_some(best)
}
