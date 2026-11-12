//! Client behaviour over a mock wire: cache, offline, seeds, rate limit, downloads.
mod common;
use common::*;
use kfx_workshop::client::*;
use kfx_workshop::model::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn cfg(s: &Scratch) -> ClientConfig {
    let mut c = ClientConfig::new(s.join("cache"));
    c.min_interval = Duration::ZERO;
    c
}

fn browse_html() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/browse-creature-p1.html"
    ))
    .unwrap()
}

#[test]
fn browse_is_cached_and_served_offline() {
    let s = Scratch::new("cl-cache");
    let t = Arc::new(MockTransport::default());
    t.add(
        "/workshop/browse?order_by=highest-rated&category=45",
        browse_html(),
    );
    let c = Client::with_transport(cfg(&s), t.clone());
    let q = BrowseQuery {
        category: Some(Category::Creature),
        ..Default::default()
    };
    let a = c.browse(&q, Policy::PreferCache).unwrap();
    assert_eq!(a.source, Source::Network);
    assert_eq!(a.value.items.len(), 5);
    let b = c.browse(&q, Policy::PreferCache).unwrap();
    assert_eq!(b.source, Source::Cache);
    assert_eq!(
        t.log.lock().unwrap().len(),
        1,
        "second call must not touch the network"
    );
    // a brand new client with the same cache dir, offline
    let mut oc = cfg(&s);
    oc.offline = true;
    let c2 = Client::with_transport(oc, Arc::new(MockTransport::default()));
    assert_eq!(
        c2.browse(&q, Policy::PreferCache)
            .unwrap()
            .value
            .items
            .len(),
        5
    );
    // not cached -> a clear error
    let other = BrowseQuery {
        category: Some(Category::Mod),
        ..Default::default()
    };
    let e = c2.browse(&other, Policy::PreferCache).unwrap_err();
    assert!(matches!(e, kfx_workshop::Error::Offline(_)), "{e}");
    assert!(e.to_string().contains("offline"));
    // Refresh while offline serves the stale cache instead of failing
    assert_eq!(
        c2.browse(&q, Policy::Refresh).unwrap().source,
        Source::StaleCache
    );
}

#[test]
fn http_errors_are_typed_and_stale_cache_survives_them() {
    let s = Scratch::new("cl-err");
    let t = Arc::new(MockTransport::default());
    let c = Client::with_transport(cfg(&s), t.clone());
    let q = BrowseQuery::default();
    let e = c.browse(&q, Policy::PreferCache).unwrap_err();
    assert!(
        matches!(e, kfx_workshop::Error::Http { status: 404, .. }),
        "{e}"
    );
    t.add("/workshop/browse?order_by=highest-rated", browse_html());
    c.browse(&q, Policy::PreferCache).unwrap();
    t.routes.lock().unwrap().clear();
    let r = c.browse(&q, Policy::Refresh).unwrap();
    assert_eq!(
        r.source,
        Source::StaleCache,
        "a failing refresh falls back to the cache"
    );
}

#[test]
fn rate_limit_spaces_live_requests_and_budget_stops_them() {
    let s = Scratch::new("cl-rate");
    let t = Arc::new(MockTransport::default());
    t.add("/api/v1/workshop/search?q=a", r#"{"workshop_items":[]}"#);
    t.add("/api/v1/workshop/search?q=b", r#"{"workshop_items":[]}"#);
    t.add("/api/v1/workshop/search?q=c", r#"{"workshop_items":[]}"#);
    let mut cc = cfg(&s);
    cc.min_interval = Duration::from_millis(200);
    cc.request_budget = Some(3);
    let c = Client::with_transport(cc, t);
    let start = Instant::now();
    for q in ["a", "b", "c"] {
        c.search_api(q, Policy::PreferCache).unwrap();
    }
    assert!(
        start.elapsed() >= Duration::from_millis(400),
        "3 requests need 2 gaps: {:?}",
        start.elapsed()
    );
    assert_eq!(c.live_requests(), 3);
    assert!(c
        .search_api("d", Policy::PreferCache)
        .unwrap_err()
        .to_string()
        .contains("budget"));
    assert!(c.search_api("", Policy::PreferCache).is_err());
}

#[test]
fn seed_folder_answers_item_pages_without_the_network() {
    let s = Scratch::new("cl-seed");
    let seed = s.join("seed");
    std::fs::create_dir_all(seed.join("pages")).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/item-10-mappack.html"
        ),
        seed.join("pages/item-10.html"),
    )
    .unwrap();
    let mut cc = cfg(&s);
    cc.seed_dirs = vec![seed];
    let t = Arc::new(MockTransport::default());
    let c = Client::with_transport(cc, t.clone());
    let d = c.item(10, "brorc-dungeons", Policy::PreferCache).unwrap();
    assert_eq!(d.source, Source::Seed);
    assert_eq!(d.value.files.len(), 4);
    assert!(t.log.lock().unwrap().is_empty());
    assert_eq!(c.live_requests(), 0);
}

#[test]
fn item_falls_back_to_the_json_when_the_page_fails() {
    let s = Scratch::new("cl-json");
    let t = Arc::new(MockTransport::default());
    t.add(
        "/api/v1/workshop/item/10",
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/item-api-10.json"
        ))
        .unwrap(),
    );
    let c = Client::with_transport(cfg(&s), t);
    let d = c.item(10, "x", Policy::PreferCache).unwrap();
    assert_eq!(d.value.source, "api");
    assert_eq!(d.value.files.len(), 4);
}

fn file(id: u32, name: &str) -> FileEntry {
    FileEntry {
        id,
        name: name.into(),
        url: format!("https://keeperfx.net/workshop/download/5/{id}/{name}"),
        ..Default::default()
    }
}

#[test]
fn download_progress_checksum_and_resume() {
    let s = Scratch::new("cl-dl");
    let t = Arc::new(MockTransport::default());
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    t.add("/workshop/download/5/77/a.zip", body.clone());
    let c = Client::with_transport(cfg(&s), t.clone());
    let dest = s.join("dl");
    let f = file(77, "a.zip");
    // first attempt: the connection drops after 100000 bytes
    *t.truncate_after.lock().unwrap() = Some(100_000);
    let cancel = AtomicBool::new(false);
    let err = c.download(5, &f, &dest, &|_| {}, &cancel).unwrap_err();
    assert!(err.is_transient(), "{err}");
    assert_eq!(
        std::fs::metadata(dest.join("a.zip.part")).unwrap().len(),
        100_000,
        "partial file kept"
    );
    assert!(!dest.join("a.zip").exists());
    // second attempt resumes with a Range request
    let seen = std::sync::Mutex::new(vec![]);
    let d = c
        .download(5, &f, &dest, &|p| seen.lock().unwrap().push(p), &cancel)
        .unwrap();
    assert_eq!(d.resumed_from, 100_000);
    assert_eq!(std::fs::read(&d.path).unwrap(), body);
    assert_eq!(d.size, 300_000);
    assert_eq!(d.sha256, kfx_workshop::util::sha256_hex(&body));
    assert!(std::fs::read_to_string(dest.join("a.zip.sha256"))
        .unwrap()
        .starts_with(&d.sha256));
    assert_eq!(t.log.lock().unwrap().last().unwrap().1, Some(100_000));
    let p = seen.lock().unwrap();
    assert!(p.iter().all(|p| p.total == 300_000));
    assert_eq!(p.last().unwrap().done, 300_000);
    assert!(!dest.join("a.zip.part").exists());
}

#[test]
fn server_ignoring_range_restarts_the_file() {
    let s = Scratch::new("cl-norange");
    let t = Arc::new(MockTransport::default());
    let body: Vec<u8> = (0..50_000u32).map(|i| (i % 7) as u8).collect();
    t.add("/workshop/download/5/78/b.zip", body.clone());
    t.ignore_range
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let c = Client::with_transport(cfg(&s), t);
    let dest = s.join("dl");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("b.zip.part"), b"garbage that must not survive").unwrap();
    let d = c
        .download(
            5,
            &file(78, "b.zip"),
            &dest,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(d.resumed_from, 0);
    assert_eq!(std::fs::read(&d.path).unwrap(), body);
}

#[test]
fn cancel_stops_a_download_and_keeps_the_partial_file() {
    let s = Scratch::new("cl-cancel");
    let t = Arc::new(MockTransport::default());
    t.add("/workshop/download/5/79/c.zip", vec![1u8; 500_000]);
    let c = Client::with_transport(cfg(&s), t);
    let cancel = AtomicBool::new(false);
    let r = c.download(
        5,
        &file(79, "c.zip"),
        &s.join("dl"),
        &|p| {
            if p.done > 100_000 {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst)
            }
        },
        &cancel,
    );
    assert!(matches!(r, Err(kfx_workshop::Error::Cancelled)));
    assert!(s.join("dl/c.zip.part").exists());
    assert!(!s.join("dl/c.zip").exists());
}

#[test]
fn offline_download_and_hostile_names() {
    let s = Scratch::new("cl-off");
    let c = Client::with_transport(cfg(&s), Arc::new(MockTransport::default()));
    c.set_offline(true);
    let e = c
        .download(
            5,
            &file(1, "x.zip"),
            &s.join("dl"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert!(matches!(e, kfx_workshop::Error::Offline(_)));
    assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_file_name("..\\..\\x.zip"), "x.zip");
    assert_eq!(sanitize_file_name(".hidden"), "hidden");
    assert_eq!(sanitize_file_name(""), "download.bin");
}

#[test]
fn download_uses_the_seed_copy() {
    let s = Scratch::new("cl-dlseed");
    let seed = s.join("seed/files/5/80");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("d.zip"), b"seeded").unwrap();
    let mut cc = cfg(&s);
    cc.seed_dirs = vec![s.join("seed")];
    let t = Arc::new(MockTransport::default());
    let c = Client::with_transport(cc, t.clone());
    let d = c
        .download(
            5,
            &file(80, "d.zip"),
            &s.join("dl"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(d.from_seed);
    assert_eq!(std::fs::read(d.path).unwrap(), b"seeded");
    assert!(t.log.lock().unwrap().is_empty());
}
