//! Typed Workshop data.

use serde::{Deserialize, Serialize};

/// Workshop item category (`/workshop/browse?category=<id>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Map,
    MapPack,
    Campaign,
    MultiplayerMap,
    MultiplayerMapPack,
    Mod,
    Creature,
    Application,
    Other,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Map,
        Category::MapPack,
        Category::Campaign,
        Category::MultiplayerMap,
        Category::MultiplayerMapPack,
        Category::Mod,
        Category::Creature,
        Category::Application,
        Category::Other,
    ];
    /// Site id used in `?category=` and by the JSON API.
    pub fn id(self) -> u32 {
        match self {
            Category::Map => 10,
            Category::MapPack => 15,
            Category::Campaign => 20,
            Category::MultiplayerMap => 30,
            Category::MultiplayerMapPack => 35,
            Category::Mod => 40,
            Category::Creature => 45,
            Category::Application => 50,
            Category::Other => 100,
        }
    }
    pub fn from_id(id: u32) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.id() == id)
    }
    pub fn label(self) -> &'static str {
        match self {
            Category::Map => "Map",
            Category::MapPack => "Map Pack",
            Category::Campaign => "Campaign",
            Category::MultiplayerMap => "Multiplayer Map",
            Category::MultiplayerMapPack => "Multiplayer Map Pack",
            Category::Mod => "Mod",
            Category::Creature => "Creature",
            Category::Application => "Application",
            Category::Other => "Other",
        }
    }
    pub fn from_label(s: &str) -> Option<Category> {
        let s = s.trim();
        Category::ALL
            .into_iter()
            .find(|c| c.label().eq_ignore_ascii_case(s))
    }
}

/// Sort orders of `?order_by=` (values taken from the browse page's dropdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Sort {
    Latest,
    Oldest,
    Name,
    #[default]
    HighestRated,
    HighestRatedSimple,
    LowestRatedSimple,
    MostDifficult,
    LeastDifficult,
    MostDownloaded,
    LeastDownloaded,
    LastUpdated,
}

impl Sort {
    pub const ALL: [Sort; 11] = [
        Sort::HighestRated,
        Sort::Latest,
        Sort::LastUpdated,
        Sort::MostDownloaded,
        Sort::Name,
        Sort::Oldest,
        Sort::HighestRatedSimple,
        Sort::LowestRatedSimple,
        Sort::MostDifficult,
        Sort::LeastDifficult,
        Sort::LeastDownloaded,
    ];
    pub fn param(self) -> &'static str {
        match self {
            Sort::Latest => "latest",
            Sort::Oldest => "oldest",
            Sort::Name => "name",
            Sort::HighestRated => "highest-rated",
            Sort::HighestRatedSimple => "highest-rated-simple",
            Sort::LowestRatedSimple => "lowest-rated-simple",
            Sort::MostDifficult => "most-difficult",
            Sort::LeastDifficult => "least-difficult",
            Sort::MostDownloaded => "most-downloaded",
            Sort::LeastDownloaded => "least-downloaded",
            Sort::LastUpdated => "last-updated",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Sort::Latest => "Latest",
            Sort::Oldest => "Oldest",
            Sort::Name => "Name",
            Sort::HighestRated => "Highest rated",
            Sort::HighestRatedSimple => "Highest rated (simple)",
            Sort::LowestRatedSimple => "Lowest rated (simple)",
            Sort::MostDifficult => "Most difficult",
            Sort::LeastDifficult => "Least difficult",
            Sort::MostDownloaded => "Most downloaded",
            Sort::LeastDownloaded => "Least downloaded",
            Sort::LastUpdated => "Last updated",
        }
    }
}

/// A browse request.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct BrowseQuery {
    pub category: Option<Category>,
    pub text: String,
    pub sort: Sort,
    /// 1-based page (50 items per page on the live site).
    pub page: u32,
}

impl BrowseQuery {
    pub fn page(&self) -> u32 {
        self.page.max(1)
    }
    /// Path and query string of the browse page.
    pub fn path(&self) -> String {
        let mut q = vec![format!("order_by={}", self.sort.param())];
        if let Some(c) = self.category {
            q.push(format!("category={}", c.id()));
        }
        if !self.text.trim().is_empty() {
            q.push(format!(
                "search={}",
                crate::util::url_encode(self.text.trim())
            ));
        }
        if self.page() > 1 {
            q.push(format!("page={}", self.page()));
        }
        format!("/workshop/browse?{}", q.join("&"))
    }
}

/// One card of a browse page.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ItemSummary {
    pub id: u32,
    pub slug: String,
    pub title: String,
    pub category: Option<Category>,
    pub submitter: String,
    /// Site-relative thumbnail path (`/workshop/image/<id>/<file>`), when the item has one.
    pub thumbnail: Option<String>,
    /// Quality rating 0..=5.
    pub rating: Option<f32>,
    /// Difficulty rating 0..=5.
    pub difficulty: Option<f32>,
    /// Filled from the JSON search API only.
    pub downloads: Option<u32>,
    pub updated: Option<String>,
    pub description: Option<String>,
    pub min_game_build: Option<i32>,
}

/// A downloadable file of an item (a row of the Downloads table).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: u32,
    pub name: String,
    pub url: String,
    pub mirrors: Vec<String>,
    /// Size in bytes: exact when it comes from the JSON API, otherwise decoded from the table's
    /// rounded label (`312.93KiB`).
    pub size: u64,
    /// Size label as shown by the site.
    pub size_label: String,
    pub downloads: u32,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Struck through and marked "(Broken)" on the site.
    pub broken: bool,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ItemDetail {
    pub id: u32,
    pub slug: String,
    pub title: String,
    pub category: Option<Category>,
    pub submitter: String,
    pub description: String,
    pub install_instructions: String,
    pub images: Vec<String>,
    pub rating: Option<f32>,
    pub rating_count: Option<u32>,
    pub difficulty: Option<f32>,
    pub difficulty_count: Option<u32>,
    pub original_author: Option<String>,
    pub original_date: Option<String>,
    pub submitted: Option<String>,
    pub updated: Option<String>,
    /// Compatibility note, e.g. `KeeperFX 1.3.2` (row "Min. game version").
    pub min_game_version: Option<String>,
    pub downloads: u32,
    /// Newest first, as on the site.
    pub files: Vec<FileEntry>,
    /// Every other label/value row of the info table (kept so a new site field is not lost).
    pub extra: Vec<(String, String)>,
    /// Where the data came from: `html`, `api`, `cache`.
    pub source: String,
}

impl ItemDetail {
    /// The newest file that is not marked broken.
    pub fn latest_file(&self) -> Option<&FileEntry> {
        self.files
            .iter()
            .filter(|f| !f.broken)
            .max_by_key(|f| (f.date.clone(), f.id))
    }
    pub fn page_url(&self, base: &str) -> String {
        format!(
            "{}/workshop/item/{}/{}",
            base.trim_end_matches('/'),
            self.id,
            self.slug
        )
    }
}

/// A page of browse results.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<ItemSummary>,
    pub page: u32,
    pub total_pages: u32,
}

/// Download progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: u64,
    /// 0 when unknown.
    pub total: u64,
    /// The download continued a partial file from this offset.
    pub resumed_from: u64,
}
