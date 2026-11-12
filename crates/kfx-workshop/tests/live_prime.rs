//! Fills a Workshop HTTP cache with a few live answers (browse page, thumbnails, a few
//! screenshots) through the real client, at one request per second and under a hard budget of
//! 24 requests. The offline screenshot test (`screens.rs`) reads that cache. Read-only GETs, no
//! login.
//!
//! This test makes network requests to keeperfx.net, so it is opt-in twice: it is `#[ignore]`d
//! and it only runs when `KFX_WORKSHOP_CACHE` names the cache folder to fill:
//!
//!   KFX_WORKSHOP_CACHE=<folder> cargo test -p kfx-workshop --test live_prime -- --ignored
use kfx_workshop::client::*;
use kfx_workshop::model::*;

#[test]
#[ignore]
fn prime_cache_for_screenshots() {
    let Some(cache) = std::env::var_os("KFX_WORKSHOP_CACHE") else {
        eprintln!("KFX_WORKSHOP_CACHE unset; skipped");
        return;
    };
    let mut cfg = ClientConfig::new(cache);
    cfg.user_agent = "kfx-editor-dev/0.1 (workshop client development)".into();
    cfg.request_budget = Some(24);
    let c = Client::new(cfg);
    let q = BrowseQuery {
        category: Some(Category::Map),
        ..Default::default()
    };
    let page = c.browse(&q, Policy::PreferCache).unwrap();
    println!(
        "browse: {} items, {} pages, source {:?}",
        page.value.items.len(),
        page.value.total_pages,
        page.source
    );
    let mut n = 0;
    for it in page.value.items.iter().take(14) {
        if let Some(t) = &it.thumbnail {
            match c.image(t, Policy::PreferCache) {
                Ok(f) => {
                    n += 1;
                    println!("thumb {} {} bytes {:?}", it.id, f.value.len(), f.source);
                }
                Err(e) => println!("thumb {}: {e}", it.id),
            }
        }
    }
    for (id, slug, take) in [
        (164u32, "my-pet-dungeon", 1usize),
        (647, "maiden-of-the-nest", 2),
    ] {
        let d = c.item(id, slug, Policy::PreferCache).unwrap();
        for im in d.value.images.iter().take(take) {
            let r = c.image(im, Policy::PreferCache);
            println!("image {id} {im}: {:?}", r.map(|f| f.value.len()));
        }
    }
    println!("thumbnails {n}; live requests made: {}", c.live_requests());
}
