//! The editor's use of `kfx-update`: settings, the updater for this build and install, and the
//! test hooks. The window's part is in `app/updates.rs`, the command line in `update_cli.rs`.
//!
//! Test hooks, honoured only by debug builds (`debug_assertions`; release builds ignore them):
//! `KFX_UPDATE_TEST_API=http://127.0.0.1:PORT` points the client at a mock GitHub over plain
//! HTTP, and `KFX_UPDATE_TEST_ALLOW_DEV=1` lets a debug build install an update (the end-to-end
//! test of `tests/update_e2e.rs`).

use kfx_update::client::{CheckRequest, Client, ClientConfig, Update};
use kfx_update::install::{detect, eligibility, Blocker, BuildInfo, FsProbe, Layout};
use kfx_update::platform::Platform;
use kfx_update::state::StateFiles;
use kfx_update::transport::UrlPolicy;
use kfx_update::updater::Updater;
use kfx_update::verify::HttpsOnly;
use kfx_update::Channel;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Preferences > Updates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateSettings {
    /// Look for a new version once a day (only after the first-run notice was acknowledged).
    pub auto_check: bool,
    pub channel: Channel,
    /// Download a new version by itself and install it when the editor is closed (off: ask).
    pub auto_install: bool,
    /// The first-run notice about update checks was shown and answered.
    pub notice_acknowledged: bool,
    /// Versions the user chose to skip.
    pub skipped: Vec<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings {
            auto_check: true,
            channel: Channel::default(),
            auto_install: false,
            notice_acknowledged: false,
            skipped: vec![],
        }
    }
}

pub fn current_version() -> semver::Version {
    semver::Version::parse(crate::VERSION).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

/// Debug builds only: a test hook from the environment.
fn test_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    } else {
        None
    }
}

pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: crate::VERSION.to_string(),
        debug: cfg!(debug_assertions) && test_env("KFX_UPDATE_TEST_ALLOW_DEV").is_none(),
    }
}

/// The client configuration: GitHub, or (debug builds) the test server of `KFX_UPDATE_TEST_API`.
pub fn client_config() -> ClientConfig {
    let mut c = ClientConfig::github(&ClientConfig::user_agent_for(crate::VERSION));
    if let Some(base) = test_env("KFX_UPDATE_TEST_API") {
        if let Some((_, authority)) = kfx_update::transport::split_url(&base) {
            c.policy = UrlPolicy::local_test_server(authority);
            c.api_base = base.trim_end_matches('/').to_string();
        }
    }
    c
}

/// The update state folder (the editor's configuration folder).
pub fn state_dir() -> Option<PathBuf> {
    crate::settings::config_dir()
}

/// The updater of this build, with the real wire.
pub fn updater() -> Option<Updater> {
    Some(Updater {
        client: Client::new(client_config()),
        files: StateFiles::new(state_dir()?),
        platform: Platform::current(),
        build: build_info(),
        verifier: Box::new(HttpsOnly),
    })
}

/// The layout of the running copy.
pub fn layout() -> Result<Layout, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok(detect(&exe, Platform::current(), &FsProbe))
}

/// Why this copy cannot replace itself, if it cannot.
pub fn blocker(layout: &Result<Layout, String>) -> Option<Blocker> {
    match layout {
        Ok(l) => eligibility(l, &build_info(), Platform::current(), &FsProbe).err(),
        Err(e) => Some(Blocker::UnknownLocation(e.clone())),
    }
}

pub fn request(s: &UpdateSettings, manual: bool) -> CheckRequest {
    CheckRequest {
        current: current_version(),
        channel: s.channel,
        skipped: if manual { vec![] } else { s.skipped.clone() },
        platform: Platform::current(),
        manual,
    }
}

/// The release page of an update, when it is a page of the editor's repository (the link is
/// shown, and opened only on request).
pub fn release_page(u: &Update) -> String {
    let base = format!(
        "https://github.com/{}/releases",
        kfx_update::client::DEFAULT_REPO
    );
    let page = &u.release.html_url;
    if page.starts_with(&format!("{base}/")) && !page.chars().any(|c| c.is_control() || c == ' ') {
        page.clone()
    } else {
        base
    }
}

/// Release notes for display: untrusted text from GitHub, made safe the way Workshop text is
/// (control and bidirectional characters replaced), capped in length, with Markdown reduced to
/// plain text: headings and emphasis lose their markers, list items get a bullet, and links show
/// their address in brackets (nothing is clickable).
pub fn notes_text(body: &str) -> String {
    const MAX: usize = 20_000;
    let body = kfx_workshop::ui::safe_text(body);
    let mut out = String::new();
    for line in body.lines() {
        let t = line.trim_end();
        let t = t.trim_start_matches('#').trim_start();
        let t = if let Some(r) = t
            .strip_prefix("* ")
            .or_else(|| t.strip_prefix("- "))
            .or_else(|| t.strip_prefix("+ "))
        {
            format!("• {r}")
        } else {
            t.to_string()
        };
        out.push_str(&plain_links(&t.replace("**", "").replace('`', "")));
        out.push('\n');
        if out.len() > MAX {
            let cut = (0..=MAX)
                .rev()
                .find(|&i| out.is_char_boundary(i))
                .unwrap_or(0);
            out.truncate(cut);
            out.push_str("\n…");
            break;
        }
    }
    out.trim_end().to_string()
}

/// `[text](url)` -> `text (url)`.
fn plain_links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else {
            break;
        };
        out.push_str(&rest[..open]);
        let text = &rest[open + 1..close];
        let url = &rest[close + 2..end];
        if text == url || text.is_empty() {
            out.push_str(url);
        } else {
            out.push_str(&format!("{text} ({url})"));
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_plain_safe_text() {
        let n = notes_text(
            "## What's new\n\n* **Bold** item\n- see [the manual](https://x/y)\n\u{202e}evil\u{7}\n`code`",
        );
        assert_eq!(
            n,
            "What's new\n\n• Bold item\n• see the manual (https://x/y)\n?evil?\ncode"
        );
        let long = "x".repeat(50_000);
        assert!(notes_text(&long).chars().count() < 20_010);
        assert_eq!(plain_links("[a](b) and [c"), "a (b) and [c");
    }

    #[test]
    fn defaults_and_old_settings() {
        let s = UpdateSettings::default();
        assert!(s.auto_check && !s.auto_install && !s.notice_acknowledged);
        assert_eq!(s.channel, Channel::PreRelease);
        // the settings of 0.1.x (update_check / update_url) still load
        let old = r#"{"update_check": true, "update_url": "https://example.com"}"#;
        let st: crate::settings::Settings = serde_json::from_str(old).unwrap();
        assert_eq!(st.updates, UpdateSettings::default());
    }

    #[test]
    fn release_page_is_only_the_repository() {
        let mut u = Update {
            release: kfx_update::Release {
                tag_name: "v1.0.0".into(),
                name: None,
                body: None,
                draft: false,
                prerelease: false,
                html_url: "https://github.com/eddoww/keeperfx-editor/releases/tag/v1.0.0".into(),
                published_at: None,
                assets: vec![],
            },
            version: semver::Version::new(1, 0, 0),
            archive: None,
            setup: None,
            sums: None,
        };
        assert!(release_page(&u).ends_with("/tag/v1.0.0"));
        u.release.html_url = "https://evil.example/x".into();
        assert_eq!(
            release_page(&u),
            "https://github.com/eddoww/keeperfx-editor/releases"
        );
    }
}
