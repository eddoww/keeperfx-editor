//! The KeeperFX revision the embedded data was generated from (`data/UPSTREAM.json`).
//!
//! `tools/sync_keeperfx.py` writes the file whenever it regenerates the script tables, the
//! configuration schema, the Lua index or the sprite tables, so the editor can tell users which
//! KeeperFX its idea of "valid" comes from.
use serde::Deserialize;
use std::sync::OnceLock;

const UPSTREAM_JSON: &str = include_str!("../../../data/UPSTREAM.json");

#[derive(Debug, Default, Deserialize)]
struct RawRepo {
    #[serde(default)]
    commit: String,
    #[serde(default)]
    date: String,
}
#[derive(Debug, Default, Deserialize)]
struct RawUpstream {
    #[serde(default)]
    keeperfx: RawRepo,
    #[serde(default)]
    wiki: RawRepo,
}

/// Upstream revisions of the embedded data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataRevision {
    /// Full KeeperFX commit hash.
    pub keeperfx_commit: String,
    /// KeeperFX commit date, ISO 8601 (`2026-11-10T16:52:28+01:00`).
    pub keeperfx_date: String,
    /// Full commit hash of the KeeperFX wiki.
    pub wiki_commit: String,
}

impl DataRevision {
    /// Parse the JSON of `data/UPSTREAM.json`; a malformed file gives an empty revision.
    pub fn from_json(json: &str) -> DataRevision {
        let raw: RawUpstream = serde_json::from_str(json).unwrap_or_default();
        DataRevision {
            keeperfx_commit: raw.keeperfx.commit,
            keeperfx_date: raw.keeperfx.date,
            wiki_commit: raw.wiki.commit,
        }
    }

    /// The revision embedded in this build.
    pub fn get() -> &'static DataRevision {
        static R: OnceLock<DataRevision> = OnceLock::new();
        R.get_or_init(|| DataRevision::from_json(UPSTREAM_JSON))
    }

    /// Abbreviated KeeperFX commit (7 characters), or `unknown`.
    pub fn keeperfx_short(&self) -> &str {
        short(&self.keeperfx_commit)
    }

    /// Abbreviated wiki commit (7 characters), or `unknown`.
    pub fn wiki_short(&self) -> &str {
        short(&self.wiki_commit)
    }

    /// Day of the KeeperFX commit (`2026-11-10`), or `unknown`.
    pub fn keeperfx_day(&self) -> &str {
        match self.keeperfx_date.get(..10) {
            Some(d) if d.len() == 10 => d,
            _ => "unknown",
        }
    }

    /// URL of the KeeperFX commit on GitHub, when known.
    pub fn keeperfx_url(&self) -> Option<String> {
        (!self.keeperfx_commit.is_empty()).then(|| {
            format!(
                "https://github.com/dkfans/keeperfx/commit/{}",
                self.keeperfx_commit
            )
        })
    }

    /// One line for `--version` and logs: `KeeperFX 5858144 (2026-11-10), wiki c3461a0`.
    pub fn summary(&self) -> String {
        format!(
            "KeeperFX {} ({}), wiki {}",
            self.keeperfx_short(),
            self.keeperfx_day(),
            self.wiki_short()
        )
    }
}

fn short(hash: &str) -> &str {
    match hash.get(..7) {
        Some(s) if hash.len() >= 7 => s,
        _ => "unknown",
    }
}

/// Where newer builds of the editor are published.
pub const RELEASES_URL: &str = "https://github.com/eddoww/keeperfx-editor/releases";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_revision_is_complete() {
        let r = DataRevision::get();
        assert_eq!(r.keeperfx_commit.len(), 40, "{r:?}");
        assert!(r.keeperfx_commit.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(r.keeperfx_day().len(), 10);
        assert!(r.keeperfx_day().starts_with("20"));
        assert_eq!(r.wiki_short().len(), 7);
        assert!(r
            .summary()
            .starts_with(&format!("KeeperFX {} (", r.keeperfx_short())));
    }

    #[test]
    fn malformed_revision_is_unknown() {
        let r = DataRevision::from_json("not json");
        assert_eq!(r.keeperfx_short(), "unknown");
        assert_eq!(r.keeperfx_day(), "unknown");
        assert!(r.keeperfx_url().is_none());
    }
}
