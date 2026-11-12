//! Read-only, cached, rate-limited Workshop client.
//!
//! # Endpoints used (all `GET`, no login, no cookies kept)
//!
//! * `/workshop/browse?order_by=<sort>&category=<id>&search=<text>&page=<n>`: HTML, 50 cards per
//!   page. The only way to filter by category and to sort; parsed by [`crate::html::parse_browse`].
//! * `/api/v1/workshop/search?q=<text>`: JSON (`{"workshop_items":[...]}`, all matching items, no
//!   paging, name search only, an empty `q` is refused with `NO_SEARCH_QUERY_GIVEN`). Richer
//!   metadata (downloads, updated date, min game build, description). Used by
//!   [`Client::search_api`].
//! * `/workshop/item/<id>[/<slug>]`: HTML item page. The **authoritative** file list (the JSON item
//!   answer is stale for many uploads, `files: []`). Parsed by [`crate::html::parse_item`].
//! * `/api/v1/workshop/item/<id>`: JSON, used only as a fallback when the page cannot be fetched.
//! * `/workshop/image/<id>/<file>`: thumbnails and screenshots.
//! * `/workshop/download/<item>/<file>/<name>`: file download (supports `Range`).
//!
//! # Politeness
//!
//! At most one live request per [`ClientConfig::min_interval`] (default 1 s) across all threads, a
//! descriptive `User-Agent`, every answer cached on disk, an optional hard budget of live
//! requests ([`ClientConfig::request_budget`]), and a read-only seed folder searched before the
//! network (the survey cache layout: `pages/item-<id>.html`, `api/item-<id>.json`,
//! `files/<item>/<file>/<name>`).

use crate::error::{Error, Result};
use crate::html;
use crate::model::*;
use crate::util::{self, sha256_file};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_BASE: &str = "https://keeperfx.net";
pub const DEFAULT_USER_AGENT: &str =
    "kfx-editor/0.1 (KeeperFX map editor Workshop browser; read-only)";

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The wire. Tests and offline hosts substitute their own.
pub trait Transport: Send + Sync {
    /// `GET url`, with `Range: bytes=<from>-` when `range_from` is set. HTTP error statuses are
    /// returned as a [`Response`], not an error.
    fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response>;
}

pub struct UreqTransport {
    agent: ureq::Agent,
    user_agent: String,
}

impl UreqTransport {
    pub fn new(user_agent: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .redirects(5)
            .build();
        UreqTransport {
            agent,
            user_agent: user_agent.to_string(),
        }
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response> {
        let mut req = self
            .agent
            .get(url)
            .set("User-Agent", &self.user_agent)
            .set("Accept", "*/*");
        if let Some(r) = range_from {
            req = req.set("Range", &format!("bytes={r}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => {
                return Err(Error::Network {
                    url: url.to_string(),
                    message: e.to_string(),
                })
            }
        };
        let headers = resp
            .headers_names()
            .into_iter()
            .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
            .collect();
        let status = resp.status();
        Ok(Response {
            status,
            headers,
            body: Box::new(resp.into_reader()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub base_url: String,
    pub user_agent: String,
    /// Cache root (`http/`, `downloads/` below).
    pub cache_dir: PathBuf,
    /// Read-only folders with the survey layout, tried before the network.
    pub seed_dirs: Vec<PathBuf>,
    pub min_interval: Duration,
    pub offline: bool,
    /// Listings are served from the cache while younger than this.
    pub max_age_list: Duration,
    pub max_age_item: Duration,
    /// Refuse to make more than this many live requests in the life of the client.
    pub request_budget: Option<u32>,
}

impl ClientConfig {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        ClientConfig {
            base_url: DEFAULT_BASE.into(),
            user_agent: DEFAULT_USER_AGENT.into(),
            cache_dir: cache_dir.into(),
            seed_dirs: vec![],
            min_interval: Duration::from_secs(1),
            offline: false,
            max_age_list: Duration::from_secs(3600),
            max_age_item: Duration::from_secs(24 * 3600),
            request_budget: Some(200),
        }
    }
}

/// How hard to try the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Policy {
    /// Fresh cache, then seed, then network; a stale cache answers when the network fails.
    #[default]
    PreferCache,
    /// Skip cache freshness and the seed; fall back to the cache only when the network fails.
    Refresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Network,
    Cache,
    /// Served from the offline cache because the network failed or offline mode is on.
    StaleCache,
    Seed,
}

#[derive(Clone, Debug)]
pub struct Fetched<T> {
    pub value: T,
    pub source: Source,
}

/// A finished download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub resumed_from: u64,
    pub from_seed: bool,
}

pub struct Client {
    cfg: ClientConfig,
    transport: Arc<dyn Transport>,
    last: Mutex<Option<Instant>>,
    offline: AtomicBool,
    live: AtomicU32,
}

fn io_err(p: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |e| Error::io(p, e)
}

impl Client {
    pub fn new(cfg: ClientConfig) -> Client {
        let t = Arc::new(UreqTransport::new(&cfg.user_agent));
        Client::with_transport(cfg, t)
    }
    pub fn with_transport(cfg: ClientConfig, transport: Arc<dyn Transport>) -> Client {
        let off = cfg.offline;
        Client {
            cfg,
            transport,
            last: Mutex::new(None),
            offline: AtomicBool::new(off),
            live: AtomicU32::new(0),
        }
    }
    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }
    pub fn set_offline(&self, v: bool) {
        self.offline.store(v, Ordering::SeqCst);
    }
    pub fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }
    /// Live requests made by this client so far.
    pub fn live_requests(&self) -> u32 {
        self.live.load(Ordering::SeqCst)
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!(
                "{}{}",
                self.cfg.base_url.trim_end_matches('/'),
                if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/{path}")
                }
            )
        }
    }

    fn throttle_and_count(&self, url: &str) -> Result<()> {
        if let Some(b) = self.cfg.request_budget {
            if self.live.load(Ordering::SeqCst) >= b {
                return Err(Error::Other(format!(
                    "live request budget ({b}) exhausted before {url}"
                )));
            }
        }
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = *last {
            let el = t.elapsed();
            if el < self.cfg.min_interval {
                std::thread::sleep(self.cfg.min_interval - el);
            }
        }
        *last = Some(Instant::now());
        self.live.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    // ---- disk cache -------------------------------------------------------------------------

    fn cache_file(&self, url: &str) -> PathBuf {
        let h = util::sha256_hex(url.as_bytes());
        self.cfg.cache_dir.join("http").join(&h[..32])
    }

    fn cache_read(&self, url: &str) -> Option<(Vec<u8>, u64)> {
        let p = self.cache_file(url);
        let body = std::fs::read(p.with_extension("body")).ok()?;
        let meta = std::fs::read_to_string(p.with_extension("meta")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&meta).ok()?;
        Some((body, v.get("fetched_at")?.as_u64()?))
    }

    fn cache_write(&self, url: &str, body: &[u8]) -> Result<()> {
        let p = self.cache_file(url);
        let dir = p.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        let meta =
            serde_json::json!({"url": url, "fetched_at": util::now_secs(), "bytes": body.len()});
        kfx_formats::atomic::atomic_write(&p.with_extension("body"), body).map_err(io_err(&p))?;
        kfx_formats::atomic::atomic_write(&p.with_extension("meta"), meta.to_string().as_bytes())
            .map_err(io_err(&p))?;
        Ok(())
    }

    /// Seed lookup for a URL path.
    fn seed_read(&self, url: &str) -> Option<Vec<u8>> {
        let path = url
            .strip_prefix(self.cfg.base_url.trim_end_matches('/'))
            .unwrap_or(url);
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let rel: Option<String> = if let Some(r) = path.strip_prefix("/workshop/item/") {
            r.split('/')
                .next()
                .and_then(|id| id.parse::<u32>().ok())
                .map(|id| format!("pages/item-{id}.html"))
        } else if let Some(r) = path.strip_prefix("/api/v1/workshop/item/") {
            r.split('/')
                .next()
                .and_then(|id| id.parse::<u32>().ok())
                .map(|id| format!("api/item-{id}.json"))
        } else {
            None
        };
        let rel = rel?;
        self.cfg
            .seed_dirs
            .iter()
            .find_map(|d| std::fs::read(d.join(&rel)).ok())
    }

    fn fetch_live(&self, url: &str) -> Result<Vec<u8>> {
        self.throttle_and_count(url)?;
        let resp = self.transport.get(url, None)?;
        if resp.status != 200 {
            return Err(Error::Http {
                status: resp.status,
                url: url.to_string(),
            });
        }
        let mut body = vec![];
        resp.body
            .take(64 * 1024 * 1024)
            .read_to_end(&mut body)
            .map_err(|e| Error::Network {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        Ok(body)
    }

    /// Cached GET. See [`Policy`].
    pub fn get_bytes(
        &self,
        path: &str,
        max_age: Duration,
        policy: Policy,
    ) -> Result<Fetched<Vec<u8>>> {
        let url = self.url(path);
        let cached = self.cache_read(&url);
        if policy == Policy::PreferCache {
            if let Some((b, at)) = &cached {
                if self.is_offline() || util::now_secs().saturating_sub(*at) <= max_age.as_secs() {
                    return Ok(Fetched {
                        value: b.clone(),
                        source: Source::Cache,
                    });
                }
            }
            if let Some(b) = self.seed_read(&url) {
                return Ok(Fetched {
                    value: b,
                    source: Source::Seed,
                });
            }
        }
        if self.is_offline() {
            return match cached {
                Some((b, _)) => Ok(Fetched {
                    value: b,
                    source: Source::StaleCache,
                }),
                None => Err(Error::Offline(url)),
            };
        }
        match self.fetch_live(&url) {
            Ok(b) => {
                self.cache_write(&url, &b)?;
                Ok(Fetched {
                    value: b,
                    source: Source::Network,
                })
            }
            Err(e) => match cached {
                Some((b, _)) if e.is_transient() || matches!(e, Error::Http { .. }) => {
                    Ok(Fetched {
                        value: b,
                        source: Source::StaleCache,
                    })
                }
                _ => Err(e),
            },
        }
    }

    fn get_text(&self, path: &str, max_age: Duration, policy: Policy) -> Result<Fetched<String>> {
        let f = self.get_bytes(path, max_age, policy)?;
        Ok(Fetched {
            value: String::from_utf8_lossy(&f.value).into_owned(),
            source: f.source,
        })
    }

    // ---- typed calls ------------------------------------------------------------------------

    /// One page of the browse listing.
    pub fn browse(&self, q: &BrowseQuery, policy: Policy) -> Result<Fetched<Page>> {
        let f = self.get_text(&q.path(), self.cfg.max_age_list, policy)?;
        let mut page = html::parse_browse(&f.value)?;
        page.page = q.page();
        Ok(Fetched {
            value: page,
            source: f.source,
        })
    }

    /// Name search through the JSON API (all matches, richer metadata, no category filter).
    pub fn search_api(&self, text: &str, policy: Policy) -> Result<Fetched<Vec<ItemSummary>>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Other(
                "the Workshop search API refuses an empty query".into(),
            ));
        }
        let f = self.get_text(
            &format!("/api/v1/workshop/search?q={}", util::url_encode(text)),
            self.cfg.max_age_list,
            policy,
        )?;
        Ok(Fetched {
            value: html::parse_search_api(&f.value)?,
            source: f.source,
        })
    }

    /// Item details from the HTML page (authoritative files); falls back to the JSON item.
    pub fn item(&self, id: u32, slug: &str, policy: Policy) -> Result<Fetched<ItemDetail>> {
        let path = if slug.is_empty() {
            format!("/workshop/item/{id}")
        } else {
            format!("/workshop/item/{id}/{slug}")
        };
        match self.get_text(&path, self.cfg.max_age_item, policy) {
            Ok(f) => {
                let mut d = html::parse_item(&f.value)?;
                if d.id == 0 {
                    d.id = id;
                }
                if d.slug.is_empty() {
                    d.slug = slug.to_string();
                }
                Ok(Fetched {
                    value: d,
                    source: f.source,
                })
            }
            Err(e @ Error::Offline(_)) => Err(e),
            Err(e) => {
                // JSON fallback (cached or live)
                match self.get_text(
                    &format!("/api/v1/workshop/item/{id}"),
                    self.cfg.max_age_item,
                    policy,
                ) {
                    Ok(j) => {
                        let mut d = html::parse_item_api(&j.value, id)?;
                        d.slug = slug.to_string();
                        d.source = "api".into();
                        Ok(Fetched {
                            value: d,
                            source: j.source,
                        })
                    }
                    Err(_) => Err(e),
                }
            }
        }
    }

    /// Thumbnail / screenshot bytes (site-relative path or absolute URL).
    pub fn image(&self, path: &str, policy: Policy) -> Result<Fetched<Vec<u8>>> {
        self.get_bytes(path, Duration::from_secs(30 * 24 * 3600), policy)
    }

    // ---- downloads --------------------------------------------------------------------------

    fn seed_file(&self, item: u32, file: &FileEntry) -> Option<PathBuf> {
        self.cfg
            .seed_dirs
            .iter()
            .map(|d| {
                d.join("files")
                    .join(item.to_string())
                    .join(file.id.to_string())
                    .join(&file.name)
            })
            .find(|p| p.is_file())
    }

    /// Download `file` of `item` into `dest_dir`.
    ///
    /// * a seed copy is used when present (no network);
    /// * a partial `<name>.part` is resumed with a `Range` request (a server that ignores `Range`
    ///   restarts the file);
    /// * the finished file's SHA-256 is returned and stored next to it as `<name>.sha256`;
    /// * `cancel` is polled between chunks; the partial file is kept so the download can resume.
    pub fn download(
        &self,
        item: u32,
        file: &FileEntry,
        dest_dir: &Path,
        progress: &dyn Fn(Progress),
        cancel: &AtomicBool,
    ) -> Result<Downloaded> {
        let name = sanitize_file_name(&file.name);
        std::fs::create_dir_all(dest_dir).map_err(io_err(dest_dir))?;
        let dest = dest_dir.join(&name);
        let part = dest_dir.join(format!("{name}.part"));
        let finish = |size: u64, resumed_from: u64, from_seed: bool| -> Result<Downloaded> {
            let sha = sha256_file(&dest).map_err(io_err(&dest))?;
            let _ = std::fs::write(
                dest_dir.join(format!("{name}.sha256")),
                format!("{sha}  {name}\n"),
            );
            Ok(Downloaded {
                path: dest.clone(),
                size,
                sha256: sha,
                resumed_from,
                from_seed,
            })
        };
        if let Some(seed) = self.seed_file(item, file) {
            let size = std::fs::metadata(&seed).map_err(io_err(&seed))?.len();
            std::fs::copy(&seed, &part).map_err(io_err(&seed))?;
            std::fs::rename(&part, &dest).map_err(io_err(&dest))?;
            progress(Progress {
                done: size,
                total: size,
                resumed_from: 0,
            });
            return finish(size, 0, true);
        }
        let url = self.url(&file.url);
        if self.is_offline() {
            return Err(Error::Offline(url));
        }
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        self.throttle_and_count(&url)?;
        let mut resp = self
            .transport
            .get(&url, if have > 0 { Some(have) } else { None })?;
        let (mut out, mut done, total, resumed) = match resp.status {
            206 => {
                let total = resp
                    .header("content-range")
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&part)
                    .map_err(io_err(&part))?;
                (f, have, total, have)
            }
            200 => {
                let total = resp
                    .header("content-length")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                (
                    std::fs::File::create(&part).map_err(io_err(&part))?,
                    0,
                    total,
                    0,
                )
            }
            416 => {
                // the partial file is already complete (or bogus): accept when the size matches
                let _ = std::fs::remove_file(&part);
                return Err(Error::Http { status: 416, url });
            }
            s => return Err(Error::Http { status: s, url }),
        };
        let mut buf = vec![0u8; 64 * 1024];
        progress(Progress {
            done,
            total,
            resumed_from: resumed,
        });
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Cancelled);
            }
            let n = resp.body.read(&mut buf).map_err(|e| Error::Network {
                url: url.clone(),
                message: e.to_string(),
            })?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(io_err(&part))?;
            done += n as u64;
            progress(Progress {
                done,
                total,
                resumed_from: resumed,
            });
        }
        out.flush().map_err(io_err(&part))?;
        drop(out);
        if total > 0 && done != total {
            return Err(Error::Network {
                url,
                message: format!(
                    "connection ended after {done} of {total} bytes (partial file kept for resume)"
                ),
            });
        }
        std::fs::rename(&part, &dest).map_err(io_err(&dest))?;
        finish(done, resumed, false)
    }
}

/// A plain file name: no separators, no leading dots, never empty.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_start_matches('.').trim().to_string();
    if s.is_empty() {
        "download.bin".into()
    } else {
        s
    }
}

/// An in-memory transport for tests and demos: URL (full or path) to a body. Records requests.
#[derive(Default)]
pub struct MockTransport {
    pub routes: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    pub log: Mutex<Vec<(String, Option<u64>)>>,
    /// Make the body end early after this many bytes (to test resume).
    pub truncate_after: Mutex<Option<usize>>,
    pub ignore_range: AtomicBool,
}

impl MockTransport {
    pub fn add(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .unwrap()
            .insert(url.to_string(), body.into());
    }
}

impl Transport for MockTransport {
    fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response> {
        self.log.lock().unwrap().push((url.to_string(), range_from));
        let body = self
            .routes
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| url.ends_with(k.as_str()))
            .map(|(_, v)| v.clone());
        let Some(body) = body else {
            return Ok(Response {
                status: 404,
                headers: vec![],
                body: Box::new(std::io::empty()),
            });
        };
        let total = body.len();
        let (status, mut data, mut headers) = match range_from {
            Some(r) if !self.ignore_range.load(Ordering::SeqCst) && (r as usize) < total => (
                206,
                body[r as usize..].to_vec(),
                vec![(
                    "Content-Range".to_string(),
                    format!("bytes {r}-{}/{total}", total - 1),
                )],
            ),
            _ => (
                200,
                body,
                vec![("Content-Length".to_string(), total.to_string())],
            ),
        };
        let full = data.len();
        if let Some(n) = self.truncate_after.lock().unwrap().take() {
            data.truncate(n);
        }
        headers.push(("X-Full-Len".into(), full.to_string()));
        Ok(Response {
            status,
            headers,
            body: Box::new(std::io::Cursor::new(data)),
        })
    }
}

impl Client {
    /// Store an answer in the disk cache as if it had been fetched (for tests, demos and bundled
    /// offline data).
    pub fn seed_cache(&self, path: &str, body: &[u8]) -> Result<()> {
        self.cache_write(&self.url(path), body)
    }
}
