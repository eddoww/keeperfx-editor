// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
//! A mock GitHub on the loopback interface (plain HTTP/1.1, one thread per connection), fake
//! release packages, and scratch folders.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = kfx_formats::testutil::scratch_dir("update")
            .join(format!("{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    pub fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A logged request.
#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub headers: HashMap<String, String>,
}

/// How the server misbehaves.
#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Close asset downloads after this many bytes (Content-Length still announces the full
    /// size).
    pub truncate_at: Option<usize>,
    /// Answer the release list with `403` and an exhausted rate limit resetting at this time.
    pub rate_limited_until: Option<u64>,
    /// Ignore `Range` and always answer `200` with the whole file.
    pub no_ranges: bool,
}

#[derive(Default)]
pub struct State {
    /// Path (with query) -> (content type, body).
    pub files: HashMap<String, (String, Vec<u8>)>,
    /// Path -> redirect target (absolute URL or path).
    pub redirects: HashMap<String, String>,
    pub etag: Option<String>,
    pub faults: Faults,
    pub log: Vec<Seen>,
}

pub struct MockServer {
    pub addr: String,
    pub state: Arc<Mutex<State>>,
}

impl MockServer {
    pub fn start() -> MockServer {
        let l = TcpListener::bind(("127.0.0.1", 0)).expect("bind the mock server");
        let addr = l.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { continue };
                let st = st.clone();
                std::thread::spawn(move || {
                    let _ = serve(s, &st);
                });
            }
        });
        MockServer { addr, state }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn put(&self, path: &str, ctype: &str, body: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_string(), (ctype.to_string(), body));
    }

    pub fn redirect(&self, from: &str, to: &str) {
        self.state
            .lock()
            .unwrap()
            .redirects
            .insert(from.to_string(), to.to_string());
    }

    pub fn faults(&self, f: Faults) {
        self.state.lock().unwrap().faults = f;
    }

    pub fn log(&self) -> Vec<Seen> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }
}

fn serve(s: TcpStream, st: &Mutex<State>) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut w = s;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let mut headers = HashMap::new();
        loop {
            let mut h = String::new();
            r.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let (status, extra, body, truncate) = {
            let mut g = st.lock().unwrap();
            g.log.push(Seen {
                path: path.clone(),
                headers: headers.clone(),
            });
            respond(&mut g, &path, &headers)
        };
        let reason = match status {
            200 => "OK",
            206 => "Partial Content",
            302 => "Found",
            304 => "Not Modified",
            403 => "Forbidden",
            404 => "Not Found",
            416 => "Range Not Satisfiable",
            _ => "Status",
        };
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in &extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        w.write_all(head.as_bytes())?;
        match truncate {
            Some(n) if n < body.len() => {
                w.write_all(&body[..n])?;
                w.flush()?;
                // drop the connection mid-body
                return Ok(());
            }
            _ => w.write_all(&body)?,
        }
        w.flush()?;
    }
}

type Answer = (u16, Vec<(String, String)>, Vec<u8>, Option<usize>);

fn respond(g: &mut State, path: &str, headers: &HashMap<String, String>) -> Answer {
    let h = |k: &str, v: &str| (k.to_string(), v.to_string());
    if let Some(to) = g.redirects.get(path) {
        return (302, vec![h("Location", to)], vec![], None);
    }
    let is_list = path.contains("/releases?");
    if is_list {
        if let Some(until) = g.faults.rate_limited_until {
            return (
                403,
                vec![
                    h("x-ratelimit-limit", "60"),
                    h("x-ratelimit-remaining", "0"),
                    h("x-ratelimit-reset", &until.to_string()),
                ],
                b"{\"message\":\"API rate limit exceeded\"}".to_vec(),
                None,
            );
        }
        if let (Some(e), Some(sent)) = (&g.etag, headers.get("if-none-match")) {
            if e == sent {
                return (
                    304,
                    vec![h("ETag", e), h("x-ratelimit-remaining", "57")],
                    vec![],
                    None,
                );
            }
        }
    }
    let Some((ctype, body)) = g.files.get(path).cloned() else {
        return (404, vec![], b"not found".to_vec(), None);
    };
    let mut extra = vec![h("Content-Type", &ctype)];
    if is_list {
        if let Some(e) = &g.etag {
            extra.push(h("ETag", e));
        }
        extra.push(h("x-ratelimit-remaining", "58"));
        return (200, extra, body, None);
    }
    extra.push(h("Accept-Ranges", "bytes"));
    if let (Some(r), false) = (headers.get("range"), g.faults.no_ranges) {
        let from: usize = r
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap_or(0);
        if from >= body.len() {
            return (416, extra, vec![], None);
        }
        extra.push(h(
            "Content-Range",
            &format!("bytes {from}-{}/{}", body.len() - 1, body.len()),
        ));
        return (206, extra, body[from..].to_vec(), g.faults.truncate_at);
    }
    (200, extra, body, g.faults.truncate_at)
}

/// A release package: `kfx-editor-<tag>-<platform>/` with the executable and a few files, as
/// tar.gz (or zip on Windows), like the release workflow makes.
pub fn make_package(out: &Path, top: &str, exe_name: &str, exe: &[u8], zip_format: bool) {
    let files: Vec<(String, Vec<u8>, bool)> = vec![
        (format!("{top}/{exe_name}"), exe.to_vec(), true),
        (format!("{top}/README.md"), b"new readme".to_vec(), false),
        (format!("{top}/LICENSE"), b"licence".to_vec(), false),
        (
            format!("{top}/docs/manual/updates.md"),
            b"new manual".to_vec(),
            false,
        ),
        // incompressible filler, so that a download is long enough to be cut in the middle
        (format!("{top}/docs/filler.bin"), filler(256 << 10), false),
    ];
    if zip_format {
        let f = std::fs::File::create(out).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, b, x) in &files {
            let o = zip::write::SimpleFileOptions::default().unix_permissions(if *x {
                0o755
            } else {
                0o644
            });
            z.start_file(n.as_str(), o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    } else {
        let f = std::fs::File::create(out).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (n, b, x) in &files {
            let mut h = tar::Header::new_gnu();
            h.set_size(b.len() as u64);
            h.set_mode(if *x { 0o755 } else { 0o644 });
            h.set_entry_type(tar::EntryType::Regular);
            t.append_data(&mut h, n, &b[..]).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }
}

/// Deterministic pseudo-random bytes.
pub fn filler(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Paths whose content differs between two snapshots (for readable assertion messages).
pub fn diff(
    a: &std::collections::BTreeMap<String, Vec<u8>>,
    b: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let mut v: Vec<String> = a
        .keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn sha256(b: &[u8]) -> String {
    kfx_update::verify::sha256_bytes(b)
}

/// The recorded live release list (trimmed), with its download URLs moved to `base`.
pub fn live_fixture(base: &str) -> String {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/releases-live.json"
    ))
    .unwrap();
    raw.replace(
        "https://github.com/eddoww/keeperfx-editor/releases/download/",
        &format!("{base}/dl/"),
    )
}

/// Snapshot of a folder: relative path -> content.
pub fn snapshot(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut m = std::collections::BTreeMap::new();
    fn walk(root: &Path, d: &Path, m: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                m.insert(format!("{rel}/"), vec![]);
                walk(root, &p, m);
            } else {
                m.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    walk(root, root, &mut m);
    m
}

/// Read a whole response body (for tests of the transport).
pub fn read_all(mut r: impl Read) -> Vec<u8> {
    let mut v = vec![];
    r.read_to_end(&mut v).unwrap();
    v
}
