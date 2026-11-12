//! The GitHub client: the release list (with ETag, rate-limit and back-off handling), small
//! assets and resumable downloads. Every request goes through the [`UrlPolicy`], redirects
//! included.

use crate::platform::{select_asset, PackageKind, Platform, SUMS_NAME};
use crate::release::{parse_releases, select, Asset, Channel, Release};
use crate::state::{rate_limit_until, CheckState, StateFiles};
use crate::transport::{resolve_location, HttpTransport, Response, Transport, UrlPolicy};
use crate::{Error, Result};
use semver::Version;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The repository whose releases are the editor's updates.
pub const DEFAULT_REPO: &str = "eddoww/keeperfx-editor";
pub const DEFAULT_API: &str = "https://api.github.com";
/// Largest release list accepted.
const MAX_LIST_BYTES: u64 = 8 << 20;

#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// `owner/repo`.
    pub repo: String,
    /// `https://api.github.com` (a test server in tests).
    pub api_base: String,
    pub user_agent: String,
    pub policy: UrlPolicy,
}

impl ClientConfig {
    /// The editor's repository on GitHub, pinned to GitHub's hosts.
    pub fn github(user_agent: &str) -> Self {
        ClientConfig {
            repo: DEFAULT_REPO.into(),
            api_base: DEFAULT_API.into(),
            user_agent: user_agent.into(),
            policy: UrlPolicy::github(),
        }
    }

    /// A descriptive User-Agent for `version` of the editor.
    pub fn user_agent_for(version: &str) -> String {
        format!(
            "kfx-editor/{version} (KeeperFX Map Editor update check; +https://github.com/{DEFAULT_REPO})"
        )
    }
}

pub struct Client {
    cfg: ClientConfig,
    transport: Arc<dyn Transport>,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckOutcome {
    /// An automatic check is not due yet (a day has not passed, or GitHub asked to wait).
    NotDue,
    /// No newer release on the channel (`latest` is the highest version seen, if any).
    UpToDate {
        latest: Option<Version>,
    },
    Available(Box<Update>),
}

/// A newer release and its files for this platform.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub release: Release,
    pub version: Version,
    /// The archive for this platform (`None`: the release has no build for it).
    pub archive: Option<Asset>,
    /// The Windows installer, when the release has one.
    pub setup: Option<Asset>,
    /// `SHA256SUMS.txt` of the release.
    pub sums: Option<Asset>,
}

#[derive(Clone, Debug)]
pub struct CheckRequest {
    pub current: Version,
    pub channel: Channel,
    pub skipped: Vec<String>,
    pub platform: Platform,
    /// "Check now": ignores the daily interval (not a rate-limit back-off).
    pub manual: bool,
}

impl Client {
    pub fn new(cfg: ClientConfig) -> Client {
        Client::with_transport(cfg, Arc::new(HttpTransport::new()))
    }

    pub fn with_transport(cfg: ClientConfig, transport: Arc<dyn Transport>) -> Client {
        Client { cfg, transport }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }

    /// `GET url`, following up to five redirects, every hop checked against the policy.
    fn get(&self, url: &str, extra: &[(&str, String)]) -> Result<Response> {
        let mut url = url.to_string();
        for _ in 0..=5 {
            self.cfg.policy.check(&url)?;
            let mut headers: Vec<(&str, String)> =
                vec![("User-Agent", self.cfg.user_agent.clone())];
            headers.extend(extra.iter().cloned());
            let r = self.transport.get(&url, &headers)?;
            if matches!(r.status, 301 | 302 | 303 | 307 | 308) {
                let loc = r
                    .header("location")
                    .and_then(|l| resolve_location(&url, l))
                    .ok_or_else(|| Error::Http {
                        status: r.status,
                        url: url.clone(),
                    })?;
                url = loc;
                continue;
            }
            return Ok(r);
        }
        Err(Error::Network {
            url,
            message: "too many redirects".into(),
        })
    }

    fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/releases?per_page=30",
            self.cfg.api_base.trim_end_matches('/'),
            self.cfg.repo
        )
    }

    /// The release list. Sends the cached ETag (`304 Not Modified` serves the cached list),
    /// refuses to ask while backing off, and records rate limits and failures in `state` (the
    /// caller saves it).
    pub fn fetch_releases(
        &self,
        files: &StateFiles,
        state: &mut CheckState,
        now: u64,
    ) -> Result<Vec<Release>> {
        if state.backing_off(now) {
            return Err(Error::RateLimited {
                until: state.backoff_until.unwrap_or(now),
            });
        }
        let cached = files.load_cache();
        let mut headers = vec![
            ("Accept", "application/vnd.github+json".to_string()),
            ("X-GitHub-Api-Version", "2022-11-28".to_string()),
        ];
        if let (Some(etag), Some(_)) = (&state.etag, &cached) {
            headers.push(("If-None-Match", etag.clone()));
        }
        let url = self.releases_url();
        let r = match self.get(&url, &headers) {
            Ok(r) => r,
            Err(e) => {
                state.failed(now, &e, None);
                return Err(e);
            }
        };
        match r.status {
            200 => {
                let etag = r.header("etag").map(str::to_string);
                let mut body = vec![];
                let read = r
                    .body
                    .take(MAX_LIST_BYTES + 1)
                    .read_to_end(&mut body)
                    .map_err(|e| Error::Network {
                        url: url.clone(),
                        message: e.to_string(),
                    });
                if let Err(e) = read {
                    state.failed(now, &e, None);
                    return Err(e);
                }
                let list = match parse_releases(&body) {
                    Ok(l) => l,
                    Err(e) => {
                        state.failed(now, &e, None);
                        return Err(e);
                    }
                };
                // a failure to cache only costs a full answer next time
                let _ = files.save_cache(&body);
                state.succeeded(now, etag);
                Ok(list)
            }
            304 => {
                let list = cached
                    .as_deref()
                    .ok_or_else(|| Error::parse("the cached release list", "missing"))
                    .and_then(parse_releases);
                match list {
                    Ok(l) => {
                        state.succeeded(now, None);
                        Ok(l)
                    }
                    Err(e) => {
                        // forget the ETag so that the next request gets a full answer
                        state.etag = None;
                        state.failed(now, &e, None);
                        Err(e)
                    }
                }
            }
            status => {
                let until = rate_limit_until(status, &|n| r.header(n).map(str::to_string), now);
                let e = match until {
                    Some(until) => Error::RateLimited { until },
                    None => Error::Http { status, url },
                };
                state.failed(now, &e, until);
                Err(e)
            }
        }
    }

    /// Check for an update: the release list (respecting the daily interval for automatic
    /// checks), then the choice of [`select`] and the assets for the platform. State is loaded
    /// from and saved to `files`.
    pub fn check(&self, files: &StateFiles, req: &CheckRequest, now: u64) -> Result<CheckOutcome> {
        let mut state = files.load();
        if !req.manual && !state.auto_due(now) {
            return Ok(CheckOutcome::NotDue);
        }
        let res = self.fetch_releases(files, &mut state, now);
        files.save(&state)?;
        let releases = res?;
        Ok(outcome(&releases, req))
    }

    /// A small asset in memory (the sums file, a signature), at most `max` bytes.
    pub fn fetch_small(&self, asset: &Asset, max: u64) -> Result<Vec<u8>> {
        let r = self.get(
            &asset.browser_download_url,
            &[("Accept", "application/octet-stream".into())],
        )?;
        if r.status != 200 {
            return Err(Error::Http {
                status: r.status,
                url: asset.browser_download_url.clone(),
            });
        }
        let mut body = vec![];
        r.body
            .take(max + 1)
            .read_to_end(&mut body)
            .map_err(|e| Error::Network {
                url: asset.browser_download_url.clone(),
                message: e.to_string(),
            })?;
        if body.len() as u64 > max {
            return Err(Error::parse(
                "a release file",
                format!("{} is too large", asset.name),
            ));
        }
        Ok(body)
    }

    /// Download `asset` to `dest`, resuming a `dest.part` left by an earlier attempt (HTTP
    /// `Range`). `progress(done, total)` is called as data arrives; setting `cancel` stops the
    /// download (the partial file is kept for the next attempt). The size is checked; the
    /// checksum is the caller's business ([`crate::verify::check_file`]).
    pub fn download(
        &self,
        asset: &Asset,
        dest: &Path,
        progress: &mut dyn FnMut(u64, u64),
        cancel: &AtomicBool,
    ) -> Result<()> {
        let part = part_path(dest);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::io(d, e))?;
        }
        let total = asset.size;
        let url = &asset.browser_download_url;
        for attempt in 0..2 {
            let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if have > total {
                std::fs::remove_file(&part).map_err(|e| Error::io(&part, e))?;
                have = 0;
            }
            if have == total && total > 0 {
                break;
            }
            let mut headers = vec![("Accept", "application/octet-stream".to_string())];
            if have > 0 {
                headers.push(("Range", format!("bytes={have}-")));
            }
            let r = self.get(url, &headers)?;
            let append = match r.status {
                206 if have > 0 => {
                    let ok = r
                        .header("content-range")
                        .is_some_and(|c| c.trim().starts_with(&format!("bytes {have}-")));
                    if !ok {
                        return Err(Error::parse(
                            "a download",
                            format!("unexpected Content-Range for {}", asset.name),
                        ));
                    }
                    true
                }
                200 => false,
                416 if attempt == 0 => {
                    let _ = std::fs::remove_file(&part);
                    continue;
                }
                status => {
                    return Err(Error::Http {
                        status,
                        url: url.clone(),
                    })
                }
            };
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(append)
                .truncate(!append)
                .open(&part)
                .map_err(|e| Error::io(&part, e))?;
            let mut done = if append { have } else { 0 };
            progress(done, total);
            let mut body = r.body;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = f.flush();
                    return Err(Error::Cancelled);
                }
                let n = body.read(&mut buf).map_err(|e| Error::Network {
                    url: url.clone(),
                    message: e.to_string(),
                })?;
                if n == 0 {
                    break;
                }
                done += n as u64;
                if done > total {
                    drop(f);
                    let _ = std::fs::remove_file(&part);
                    return Err(Error::SizeMismatch {
                        name: asset.name.clone(),
                        expected: total,
                        actual: done,
                    });
                }
                f.write_all(&buf[..n]).map_err(|e| Error::io(&part, e))?;
                progress(done, total);
            }
            f.sync_all().map_err(|e| Error::io(&part, e))?;
            if done < total {
                // the partial file stays for a resumed attempt
                return Err(Error::Network {
                    url: url.clone(),
                    message: format!("the connection closed after {done} of {total} bytes"),
                });
            }
            break;
        }
        std::fs::rename(&part, dest).map_err(|e| Error::io(dest, e))
    }
}

/// `dest` + `.part`.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// The outcome of a check for a release list.
pub fn outcome(releases: &[Release], req: &CheckRequest) -> CheckOutcome {
    match select(releases, &req.current, req.channel, &req.skipped) {
        Some((r, v)) => CheckOutcome::Available(Box::new(Update {
            archive: select_asset(r, req.platform, PackageKind::Archive).cloned(),
            setup: select_asset(r, req.platform, PackageKind::WindowsSetup).cloned(),
            sums: r.asset(SUMS_NAME).cloned(),
            release: r.clone(),
            version: v,
        })),
        None => CheckOutcome::UpToDate {
            latest: releases
                .iter()
                .filter(|r| req.channel.includes(r))
                .filter_map(|r| r.version())
                .max(),
        },
    }
}
