//! Fixture-based parser tests, plus a parse of every cached item page.
use kfx_workshop::html::*;
use kfx_workshop::model::*;
use std::path::PathBuf;

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn item_page_map_pack_with_versions_and_a_broken_file() {
    let d = parse_item(&fx("item-10-mappack.html")).unwrap();
    assert_eq!(d.id, 10);
    assert_eq!(d.slug, "brorc-dungeons");
    assert_eq!(d.title, "Brorc Dungeons");
    assert_eq!(d.category, Some(Category::MapPack));
    assert_eq!(d.rating, Some(3.5));
    assert_eq!(d.rating_count, Some(8));
    assert_eq!(d.difficulty, Some(3.13));
    assert_eq!(d.original_date.as_deref(), Some("2021-08-05"));
    assert_eq!(d.submitted.as_deref(), Some("2023-03-31"));
    assert_eq!(d.updated.as_deref(), Some("2025-08-14"));
    assert_eq!(d.downloads, 527);
    assert!(
        d.description
            .starts_with("A collection of levels against the Heroes"),
        "{:?}",
        d.description
    );
    assert_eq!(d.install_instructions, "Unzip into the levels folder.");
    assert_eq!(d.files.len(), 4);
    let f = &d.files[0];
    assert_eq!(
        (
            f.id,
            f.name.as_str(),
            f.date.as_str(),
            f.size_label.as_str()
        ),
        (2038, "brorc.zip", "2025-08-14", "312.93KiB")
    );
    assert_eq!(
        f.url,
        "https://keeperfx.net/workshop/download/10/2038/brorc.zip"
    );
    assert_eq!(
        f.mirrors,
        vec!["https://cdn-cf1.keeperfx.net/workshop/download/10/2038/brorc.zip"]
    );
    assert_eq!(f.downloads, 102);
    assert!((f.size as i64 - 320443).abs() < 100, "{}", f.size);
    let broken = d.files.iter().find(|f| f.id == 325).unwrap();
    assert!(broken.broken);
    assert!(!d.files[0].broken);
    assert_eq!(
        d.latest_file().unwrap().id,
        2038,
        "the broken old file is never the latest"
    );
}

#[test]
fn item_page_images_and_min_game_version() {
    let d = parse_item(&fx("item-28-images-minversion.html")).unwrap();
    assert!(d.images.len() >= 2, "{:?}", d.images);
    assert!(d
        .images
        .iter()
        .all(|i| i.starts_with("/workshop/image/28/")));
    assert!(
        d.min_game_version
            .as_deref()
            .unwrap_or("")
            .starts_with("KeeperFX "),
        "{:?}",
        d.min_game_version
    );
    assert!(!d.files.is_empty());
    assert!(d.category.is_some());
}

#[test]
fn item_page_without_any_file_still_parses() {
    // the one cached item (854) whose page has no Downloads table and no button
    let d = parse_item(&fx("item-854-no-versions-table.html")).unwrap();
    assert!(d.files.is_empty());
    assert!(!d.title.is_empty());
    assert!(d
        .install_instructions
        .contains("extract it to your KeeperFX"));
}

#[test]
fn item_page_download_button_fallback() {
    let html = r#"<h2 class="workshop-item-title">T</h2><table><tr><td>Filesize</td><td>1.50KiB</td></tr><tr><td>Last updated</td><td>2025-01-02</td></tr></table>
        <a href="https://keeperfx.net/workshop/download/9/77/x.zip" class="btn btn-lg" download>Download <span>x.zip</span></a>"#;
    let d = parse_item(html).unwrap();
    assert_eq!(d.files.len(), 1);
    assert_eq!(
        (
            d.files[0].id,
            d.files[0].name.as_str(),
            d.files[0].size,
            d.files[0].date.as_str()
        ),
        (77, "x.zip", 1536, "2025-01-02")
    );
}

#[test]
fn browse_page_cards_and_pagination() {
    let p = parse_browse(&fx("browse-creature-p1.html")).unwrap();
    assert_eq!(p.items.len(), 5);
    assert_eq!(p.total_pages, 2);
    assert_eq!(p.page, 1);
    let first = &p.items[0];
    assert_eq!(
        (first.id, first.slug.as_str(), first.title.as_str()),
        (647, "maiden-of-the-nest", "Maiden of the Nest")
    );
    assert_eq!(first.category, Some(Category::Creature));
    assert_eq!(first.rating, Some(5.0));
    assert_eq!(
        first.thumbnail.as_deref(),
        Some("/workshop/image/647/df01d47278e7fe59c253098dc33dc1db-thumbnail.png")
    );
    assert_eq!(first.submitter, "ErminioLucente");
    assert!(p
        .items
        .iter()
        .all(|i| i.id > 0 && !i.title.is_empty() && i.category == Some(Category::Creature)));
}

#[test]
fn search_api_and_item_api() {
    let v = parse_search_api(&fx("search-dungeon.json")).unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].id, 6);
    assert_eq!(v[0].category, Some(Category::MapPack));
    assert!(v[0].downloads.unwrap() > 0);
    assert!(v[0].updated.as_deref().unwrap().len() == 10);
    let e = parse_search_api(r#"{"success":false,"error":"NO_SEARCH_QUERY_GIVEN"}"#).unwrap_err();
    assert!(e.to_string().contains("NO_SEARCH_QUERY_GIVEN"));
    let d = parse_item_api(&fx("item-api-10.json"), 10).unwrap();
    assert_eq!(d.files.len(), 4);
    assert_eq!(d.files[0].size, 320443, "the API has exact sizes");
}

#[test]
fn tolerant_of_garbage_and_missing_parts() {
    assert!(parse_item("<html><body>nothing here</body></html>").is_err());
    assert!(parse_item("").is_err());
    let p =
        parse_browse("<div class=\"workshop-grid-item\"><a href=\"/workshop/item/5/x\">T").unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].title, "T");
    assert!(parse_browse("<<<<>>>> &bogus; <a")
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        decode_entities("Mike&#039;s &amp; &lt;b&gt; &#x41; &unknown;"),
        "Mike's & <b> A &unknown;"
    );
}

#[test]
fn query_paths() {
    let q = BrowseQuery {
        category: Some(Category::Campaign),
        text: "dark & light".into(),
        sort: Sort::Latest,
        page: 3,
    };
    assert_eq!(
        q.path(),
        "/workshop/browse?order_by=latest&category=20&search=dark+%26+light&page=3"
    );
    assert_eq!(
        BrowseQuery::default().path(),
        "/workshop/browse?order_by=highest-rated"
    );
}

/// Every item page of the Workshop survey named by `KFX_WORKSHOP_SURVEY` (see `survey.rs`) must
/// parse; where the JSON item has files, ids and sizes must agree. Skipped without a survey.
#[test]
fn every_survey_page_parses_and_agrees_with_the_api() {
    let Some(root) = std::env::var_os("KFX_WORKSHOP_SURVEY").map(PathBuf::from) else {
        eprintln!("KFX_WORKSHOP_SURVEY unset; skipped");
        return;
    };
    if !root.join("pages").is_dir() {
        eprintln!("survey absent; skipped");
        return;
    }
    let (mut pages, mut no_files, mut api_checked, mut mismatches) = (0, vec![], 0, vec![]);
    let mut cats = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(root.join("pages")).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_prefix("item-")
            .and_then(|s| s.strip_suffix(".html"))
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let html = std::fs::read_to_string(e.path()).unwrap_or_default();
        let d = match parse_item(&html) {
            Ok(d) => d,
            Err(err) => panic!("item {id}: {err}"),
        };
        pages += 1;
        assert!(!d.title.is_empty(), "item {id} has no title");
        *cats
            .entry(d.category.map_or("?", |c| c.label()))
            .or_insert(0) += 1;
        if d.files.is_empty() {
            no_files.push(id);
        }
        if let Ok(j) = std::fs::read_to_string(root.join(format!("api/item-{id}.json"))) {
            if let Ok(a) = parse_item_api(&j, id) {
                if !a.files.is_empty() {
                    api_checked += 1;
                    let page_ids: Vec<u32> = d.files.iter().map(|f| f.id).collect();
                    for f in &a.files {
                        match d.files.iter().find(|p| p.id == f.id) {
                            None => mismatches.push(format!(
                                "item {id}: api file {} missing on the page {page_ids:?}",
                                f.id
                            )),
                            Some(p) => {
                                // the page label is rounded to 2 decimals of the unit
                                let tol = (f.size as f64 * 0.006) as i64 + 1024;
                                if (p.size as i64 - f.size as i64).abs() > tol || p.name != f.name {
                                    mismatches.push(format!(
                                        "item {id}: file {} page {}/{} vs api {}/{}",
                                        f.id, p.name, p.size, f.name, f.size
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("pages parsed: {pages}; categories: {cats:?}");
    println!("pages with no file: {no_files:?}");
    println!(
        "api comparisons: {api_checked}, mismatches: {}",
        mismatches.len()
    );
    for m in mismatches.iter().take(10) {
        println!("  {m}");
    }
    assert!(pages > 100);
    assert!(
        mismatches.len() * 50 < api_checked.max(1),
        "too many page/api mismatches ({}): first {:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(5)]
    );
}
