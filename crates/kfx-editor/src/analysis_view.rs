//! The Analysis panel's logic (no UI): which `kfx_analysis` layer is shown, the background
//! worker that recomputes the analysis off the UI thread, the findings that go to the Validate
//! panel, symmetry breakers and route names.
//!
//! The worker is one thread fed by a channel: it skips to the newest job, analyses a snapshot
//! (a clone of the document and the shared game data) and sends the result back tagged with
//! the document key it was computed for; the UI drops results whose key is no longer current.
use kfx_analysis::economy::NOBODY;
use kfx_analysis::layer::{self, Layer, LayerData, LegendEntry};
use kfx_analysis::nav::INF;
use kfx_analysis::reach::{Endpoint, Route};
use kfx_analysis::sites::owner_name;
use kfx_analysis::terrain::Class;
use kfx_analysis::Analysis;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::ItemRef;
use kfx_map::transform::SymmetryMode;
use kfx_map::validate::{Issue, Location, Severity};
use kfx_prefab::i18n::{trd, trf, trn, tx, Tx};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A per-slab layer of the Analysis panel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum LayerKind {
    #[default]
    Off,
    /// Reachability tiers of one keeper (walk / dig / cross lava / never).
    Reach,
    /// Dig time from one keeper's heart (seconds).
    Dig,
    /// Walking time from one keeper's heart (seconds, without digging).
    Walk,
    /// The keeper reaching each slab first.
    Territory,
    /// Time from the nearest hero gate (heroes with a tunneller).
    Heroes,
    /// Gold and gem slabs coloured by the keeper reaching them first.
    Gold,
    /// What each slab is for movement and digging.
    Class,
    /// Slabs breaking the chosen symmetry.
    Symmetry,
}

impl LayerKind {
    pub const ALL: [LayerKind; 9] = [
        LayerKind::Off,
        LayerKind::Reach,
        LayerKind::Dig,
        LayerKind::Walk,
        LayerKind::Territory,
        LayerKind::Heroes,
        LayerKind::Gold,
        LayerKind::Class,
        LayerKind::Symmetry,
    ];

    /// Name used by `--actions`.
    pub fn key(self) -> &'static str {
        match self {
            LayerKind::Off => "off",
            LayerKind::Reach => "reach",
            LayerKind::Dig => "dig",
            LayerKind::Walk => "walk",
            LayerKind::Territory => "territory",
            LayerKind::Heroes => "heroes",
            LayerKind::Gold => "gold",
            LayerKind::Class => "class",
            LayerKind::Symmetry => "symmetry",
        }
    }

    pub fn label(self) -> Tx {
        match self {
            LayerKind::Off => tx("kfx_editor.no_layer", "No layer"),
            LayerKind::Reach => tx("kfx_editor.reachability", "Reachability"),
            LayerKind::Dig => tx("kfx_editor.dig_distance", "Dig distance"),
            LayerKind::Walk => tx("kfx_editor.walk_distance", "Walk distance"),
            LayerKind::Territory => tx("kfx_editor.territory", "Territory"),
            LayerKind::Heroes => tx("kfx_editor.hero_pressure", "Hero pressure"),
            LayerKind::Gold => tx("kfx_editor.gold_and_gems", "Gold and gems"),
            LayerKind::Class => tx("kfx_editor.slab_classes", "Slab classes"),
            LayerKind::Symmetry => tx("kfx_editor.symmetry_breaks", "Symmetry breaks"),
        }
    }

    pub fn tooltip(self) -> Tx {
        match self {
            LayerKind::Off => tx("kfx_editor.show_no_heat_map", "Show no heat map."),
            LayerKind::Reach => tx(
                "kfx_editor.where_one_keeper_s_creatures_can_walk_now_after",
                "Where one keeper's creatures can walk now, after digging, only by \
                crossing lava, or never.",
            ),
            LayerKind::Dig => tx(
                "kfx_editor.seconds_for_one_keeper_s_imps_to_reach_each_slab",
                "Seconds for one keeper's imps to reach each slab, digging on the \
                way.",
            ),
            LayerKind::Walk => tx(
                "kfx_editor.seconds_for_one_keeper_s_creatures_to_walk_to_ea",
                "Seconds for one keeper's creatures to walk to each slab without \
                digging.",
            ),
            LayerKind::Territory => tx(
                "kfx_editor.the_keeper_whose_imps_reach_each_slab_first_pale",
                "The keeper whose imps reach each slab first; pale where another \
                keeper is almost as close.",
            ),
            LayerKind::Heroes => tx(
                "kfx_editor.seconds_for_heroes_from_the_nearest_hero_gate_to",
                "Seconds for heroes from the nearest hero gate to reach each slab \
                (with a tunneller).",
            ),
            LayerKind::Gold => tx(
                "kfx_editor.gold_and_gem_slabs_in_the_colour_of_the_keeper_r",
                "Gold and gem slabs in the colour of the keeper reaching them first.",
            ),
            LayerKind::Class => tx(
                "kfx_editor.what_each_slab_is_for_movement_and_digging",
                "What each slab is for movement and digging.",
            ),
            LayerKind::Symmetry => tx(
                "kfx_editor.slabs_that_differ_from_their_mirror_or_rotated_i",
                "Slabs that differ from their mirror or rotated image.",
            ),
        }
    }

    pub fn parse(s: &str) -> Option<LayerKind> {
        LayerKind::ALL
            .iter()
            .copied()
            .find(|k| k.key().eq_ignore_ascii_case(s.trim()))
    }

    /// The layer is about one keeper.
    pub fn per_keeper(self) -> bool {
        matches!(self, LayerKind::Reach | LayerKind::Dig | LayerKind::Walk)
    }
}

/// Tabs of the Analysis panel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AnalysisTab {
    #[default]
    Layers,
    Fairness,
    Economy,
    Routes,
    Symmetry,
    Heroes,
}

impl AnalysisTab {
    pub const ALL: [AnalysisTab; 6] = [
        AnalysisTab::Layers,
        AnalysisTab::Fairness,
        AnalysisTab::Economy,
        AnalysisTab::Routes,
        AnalysisTab::Symmetry,
        AnalysisTab::Heroes,
    ];
    /// English name (also what `parse` reads).
    pub fn label(self) -> &'static str {
        self.title().en
    }
    /// The name shown in the panel.
    pub fn title(self) -> Tx {
        match self {
            AnalysisTab::Layers => tx("kfx_editor.layers", "Layers"),
            AnalysisTab::Fairness => tx("kfx_editor.fairness", "Fairness"),
            AnalysisTab::Economy => tx("kfx_editor.economy", "Economy"),
            AnalysisTab::Routes => tx("kfx_editor.routes", "Routes"),
            AnalysisTab::Symmetry => tx("kfx_editor.symmetry", "Symmetry"),
            AnalysisTab::Heroes => tx("kfx_editor.heroes", "Heroes"),
        }
    }
    pub fn parse(s: &str) -> Option<AnalysisTab> {
        AnalysisTab::ALL
            .iter()
            .copied()
            .find(|t| t.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// Persisted state of the Analysis panel (`Layout::analysis` of the settings).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AnalysisSettings {
    /// The panel is open.
    pub open: bool,
    pub tab: AnalysisTab,
    pub layer: LayerKind,
    /// Keeper of the per-keeper layers.
    pub player: u8,
    /// Opacity of the heat map, 0..255.
    pub opacity: u8,
    /// Dig routes drawn on the map.
    pub routes: bool,
    /// First-contact points drawn on the map.
    pub contacts: bool,
    /// Legend in the corner of the view.
    pub legend: bool,
    /// Findings that are clearly problems go to the Validate panel.
    pub validate: bool,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        AnalysisSettings {
            open: false,
            tab: AnalysisTab::Layers,
            layer: LayerKind::Off,
            player: 0,
            opacity: 150,
            routes: true,
            contacts: true,
            legend: true,
            validate: true,
        }
    }
}

// ------------------------------------------------------------------------------- worker

/// What an analysis was computed for: the document revision and a hash of its slabs (a newly
/// opened map starts again at revision 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocKey {
    pub revision: u64,
    pub hash: u64,
}

impl DocKey {
    pub fn of(doc: &MapDoc, revision: u64) -> DocKey {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        doc.dims.w.hash(&mut h);
        doc.dims.h.hash(&mut h);
        doc.level.hash(&mut h);
        doc.folder.hash(&mut h);
        doc.kinds.hash(&mut h);
        doc.owners.hash(&mut h);
        doc.things.items.len().hash(&mut h);
        DocKey {
            revision,
            hash: h.finish(),
        }
    }
}

struct Job {
    key: DocKey,
    doc: MapDoc,
    gd: Arc<GameData>,
}

/// A finished analysis.
pub struct Done {
    pub key: DocKey,
    pub analysis: Arc<Analysis>,
    /// Wall time of the analysis in the worker (ms).
    pub ms: f64,
}

/// The background analysis thread (started on the first job, ends with the worker).
#[derive(Default)]
pub struct Worker {
    tx: Option<Sender<Job>>,
    rx: Option<Receiver<Done>>,
    /// The key of the newest job sent and not answered yet.
    pub pending: Option<DocKey>,
}

impl Worker {
    fn start(&mut self) -> bool {
        if self.tx.is_some() {
            return true;
        }
        let (jtx, jrx) = channel::<Job>();
        let (dtx, drx) = channel::<Done>();
        let spawned = std::thread::Builder::new()
            .name("kfx-analysis".into())
            .spawn(move || {
                while let Ok(mut job) = jrx.recv() {
                    // only the newest snapshot matters
                    while let Ok(j) = jrx.try_recv() {
                        job = j;
                    }
                    let t0 = Instant::now();
                    let a = Analysis::of_doc(&job.doc, &job.gd);
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    if dtx
                        .send(Done {
                            key: job.key,
                            analysis: Arc::new(a),
                            ms,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.tx = Some(jtx);
        self.rx = Some(drx);
        true
    }

    /// Queue an analysis of a snapshot of `doc` (cloned here, on the caller's thread).
    pub fn request(&mut self, doc: &MapDoc, gd: Arc<GameData>, key: DocKey) {
        if !self.start() {
            return;
        }
        let job = Job {
            key,
            doc: doc.clone(),
            gd,
        };
        if self.tx.as_ref().is_some_and(|tx| tx.send(job).is_ok()) {
            self.pending = Some(key);
        }
    }

    /// The newest finished analysis, if any arrived.
    pub fn poll(&mut self) -> Option<Done> {
        let rx = self.rx.as_ref()?;
        let mut last = None;
        while let Ok(d) = rx.try_recv() {
            last = Some(d);
        }
        if let Some(d) = &last {
            if self.pending == Some(d.key) {
                self.pending = None;
            }
        }
        last
    }

    /// Block until the analysis of `key` arrives (headless runs and tests).
    pub fn wait(&mut self, key: DocKey, timeout: Duration) -> Option<Done> {
        let rx = self.rx.as_ref()?;
        let end = Instant::now() + timeout;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            match rx.recv_timeout(left) {
                Ok(d) if d.key == key => {
                    self.pending = None;
                    return Some(d);
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }
}

// ------------------------------------------------------------------------------- layers

/// The keeper a per-keeper layer shows: `wanted` when it has a heart, else the first keeper.
pub fn keeper_or_first(a: &Analysis, wanted: u8) -> Option<u8> {
    if a.keeper(wanted).is_some() {
        Some(wanted)
    } else {
        a.fields.first().map(|f| f.player)
    }
}

/// The symmetry mode the Symmetry layer and tab show by default: the best matching one.
pub fn best_symmetry(a: &Analysis) -> Option<SymmetryMode> {
    a.symmetry
        .iter()
        .max_by(|x, y| x.match_fraction.total_cmp(&y.match_fraction))
        .map(|c| c.mode)
}

pub fn mode_name(m: SymmetryMode) -> Tx {
    match m {
        SymmetryMode::NorthSouth => tx("kfx_editor.north_south_mirror", "North-south mirror"),
        SymmetryMode::EastWest => tx("kfx_editor.east_west_mirror", "East-west mirror"),
        SymmetryMode::FourWay => tx("kfx_editor.4_way_mirror", "4-way mirror"),
        SymmetryMode::Rot2 => tx("kfx_editor.half_turn", "Half turn"),
        SymmetryMode::Rot4 => tx("kfx_editor.quarter_turn", "Quarter turn"),
    }
}

/// Build a layer of the analysis (`None` when the map has nothing to show for it, e.g. no
/// keeper for a per-keeper layer).
pub fn build_layer(
    a: &Analysis,
    kind: LayerKind,
    player: u8,
    sym: Option<SymmetryMode>,
) -> Option<Layer> {
    let tps = a.rules.turns_per_second;
    match kind {
        LayerKind::Off => None,
        LayerKind::Reach => layer::reach_layer(a, keeper_or_first(a, player)?),
        LayerKind::Dig => {
            let p = keeper_or_first(a, player)?;
            Some(layer::distance_layer(
                &trf(
                    "kfx_editor.dig_time_from_p_heart",
                    "dig time from {p} heart",
                    &[("p", &owner_name(p))],
                ),
                &a.keeper(p)?.dig,
                tps,
            ))
        }
        LayerKind::Walk => {
            let p = keeper_or_first(a, player)?;
            Some(layer::distance_layer(
                &trf(
                    "kfx_editor.walk_time_from_p_heart",
                    "walk time from {p} heart",
                    &[("p", &owner_name(p))],
                ),
                &a.keeper(p)?.walk,
                tps,
            ))
        }
        LayerKind::Territory => (!a.fields.is_empty()).then(|| layer::territory_layer(a)),
        LayerKind::Heroes => {
            let first = a.gate_fields.first()?;
            let mut f = first.clone();
            for g in &a.gate_fields[1..] {
                for (d, e) in f.dist.iter_mut().zip(&g.dist) {
                    *d = (*d).min(*e);
                }
            }
            Some(layer::distance_layer(
                &trd(
                    "kfx_editor.hero_time_from_the_nearest_gate",
                    "hero time from the nearest gate",
                ),
                &f,
                tps,
            ))
        }
        LayerKind::Gold => {
            if a.fields.is_empty() {
                return None;
            }
            let mut legend: Vec<LegendEntry> = a
                .fields
                .iter()
                .map(|f| LegendEntry {
                    value: f.player,
                    label: owner_name(f.player).into(),
                    color: layer::owner_color(f.player),
                })
                .collect();
            legend.push(LegendEntry {
                value: 254,
                label: trd("kfx_editor.shared_tie", "shared (tie)").into(),
                color: [255, 255, 255, 255],
            });
            legend.push(LegendEntry {
                value: 253,
                label: trd("kfx_editor.nobody_reaches_it", "nobody reaches it").into(),
                color: [120, 20, 140, 255],
            });
            let values = a
                .terrain
                .cells
                .iter()
                .enumerate()
                .map(|(i, c)| match c.class {
                    Class::Gold | Class::Gems => {
                        let o = a.economy.owner_of.get(i).copied().unwrap_or(NOBODY);
                        if o != NOBODY {
                            o
                        } else if a
                            .fields
                            .iter()
                            .any(|f| f.dig.dist.get(i).is_some_and(|d| *d != INF))
                            || c.class == Class::Gems
                        {
                            254
                        } else {
                            253
                        }
                    }
                    _ => 255,
                })
                .collect();
            Some(Layer {
                name: trd(
                    "kfx_editor.gold_and_gems_by_keeper",
                    "gold and gems by keeper",
                )
                .into(),
                w: a.terrain.w,
                h: a.terrain.h,
                data: LayerData::Categorical { values, legend },
            })
        }
        LayerKind::Class => Some(layer::class_layer(&a.terrain)),
        LayerKind::Symmetry => {
            let mode = sym.or_else(|| best_symmetry(a))?;
            let check = a.symmetry.iter().find(|c| c.mode == mode)?;
            let mut values = vec![255u8; a.terrain.len()];
            for &(x, y) in &check.slab_breaks {
                if a.terrain.inside(x, y) {
                    values[a.terrain.idx(x, y)] = 1;
                }
            }
            let legend = vec![LegendEntry {
                value: 1,
                label: trd(
                    "kfx_editor.differs_from_its_image",
                    "differs from its image",
                )
                .into(),
                color: [255, 40, 200, 255],
            }];
            Some(Layer {
                name: trf(
                    "kfx_editor.mode_breaks_match_fraction_match",
                    "{mode} breaks ({match_fraction}% match)",
                    &[
                        ("mode", &mode_name(mode)),
                        (
                            "match_fraction",
                            &format!("{:.2}", check.match_fraction * 100.0),
                        ),
                    ],
                ),
                w: a.terrain.w,
                h: a.terrain.h,
                data: LayerData::Categorical { values, legend },
            })
        }
    }
}

// ------------------------------------------------------------------------------- names

pub fn endpoint_name(e: &Endpoint) -> String {
    match e {
        Endpoint::Heart { owner } => trf(
            "kfx_editor.owner_heart",
            "{owner} heart",
            &[("owner", &owner_name(*owner))],
        ),
        Endpoint::Gate { number } => trf(
            "kfx_editor.hero_gate_number",
            "hero gate {number}",
            &[("number", &number)],
        ),
        Endpoint::HeroGroup { index } => trf(
            "kfx_editor.hero_group_index",
            "hero group {index}",
            &[("index", &(index + 1))],
        ),
        Endpoint::Slab(x, y) => trf(
            "kfx_editor.slab_x_y",
            "slab ({x},{y})",
            &[("x", &x), ("y", &y)],
        ),
    }
}

pub fn seconds(a: &Analysis, turns: u32) -> String {
    if turns == INF {
        trd("kfx_editor.never", "never").into()
    } else {
        format!("{:.0} s", a.rules.seconds(turns))
    }
}

pub fn route_name(a: &Analysis, r: &Route) -> String {
    let how = match r.mover {
        kfx_analysis::nav::Mover::Siege { .. } => {
            trd("kfx_editor.breaking_walls", " (breaking walls)")
        }
        _ => String::new(),
    };
    trf(
        "kfx_editor.from_to_to_turns_how",
        "{from} to {to}: {turns}{how}",
        &[
            ("from", &endpoint_name(&r.from)),
            ("to", &endpoint_name(&r.to)),
            ("turns", &seconds(a, r.turns)),
            ("how", &how),
        ],
    )
}

// ------------------------------------------------------------------------------- symmetry

/// One thing or group of slabs breaking a symmetry, for the click-to-locate list.
#[derive(Clone, Debug, PartialEq)]
pub struct Breaker {
    pub text: String,
    pub slab: (i32, i32),
    /// The thing, when a thing breaks it.
    pub thing: Option<usize>,
}

/// What breaks `mode`: things first, then the differing slabs in 8-connected groups (largest
/// first).
pub fn breakers(a: &Analysis, mode: SymmetryMode) -> Vec<Breaker> {
    let Some(check) = a.symmetry.iter().find(|c| c.mode == mode) else {
        return vec![];
    };
    let mut out: Vec<Breaker> = check
        .thing_breaks
        .iter()
        .map(|b| {
            let thing = b
                .site
                .and_then(|s| a.sites.things.get(s))
                .and_then(|s| s.thing);
            Breaker {
                text: trf(
                    "kfx_editor.what_at_slab_slab2_no_image_at_expected_expected",
                    "{what} at ({slab},{slab2}): no image at ({expected},{expected2})",
                    &[
                        ("what", &b.what),
                        ("slab", &b.slab.0),
                        ("slab2", &b.slab.1),
                        ("expected", &b.expected.0),
                        ("expected2", &b.expected.1),
                    ],
                ),
                slab: b.slab,
                thing,
            }
        })
        .collect();
    let t = &a.terrain;
    let mut mark = vec![false; t.len()];
    for &(x, y) in &check.slab_breaks {
        if t.inside(x, y) {
            mark[t.idx(x, y)] = true;
        }
    }
    let mut groups: Vec<Vec<(i32, i32)>> = vec![];
    for i in 0..t.len() {
        if !mark[i] {
            continue;
        }
        mark[i] = false;
        let mut cells = vec![];
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            let (x, y) = t.xy(j);
            cells.push((x, y));
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if t.inside(nx, ny) && mark[t.idx(nx, ny)] {
                        mark[t.idx(nx, ny)] = false;
                        stack.push(t.idx(nx, ny));
                    }
                }
            }
        }
        groups.push(cells);
    }
    groups.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then(a[0].1.cmp(&b[0].1))
            .then(a[0].0.cmp(&b[0].0))
    });
    for g in groups {
        let c = kfx_analysis::sites::group_centre(&g);
        // a slab of the group itself nearest the centre (the centre of an L may be outside)
        let at = g
            .iter()
            .copied()
            .min_by_key(|p| (p.0 - c.0).pow(2) + (p.1 - c.1).pow(2))
            .unwrap_or(c);
        let text = if g.len() == 1 {
            trf(
                "kfx_editor.slab_at_at2_differs_from_its_image",
                "slab ({at},{at2}) differs from its image",
                &[("at", &at.0), ("at2", &at.1)],
            )
        } else {
            trf(
                "kfx_editor.g_slabs_around_at_at2_differ_from_their_images",
                "{g} slabs around ({at},{at2}) differ from their images",
                &[("g", &g.len()), ("at", &at.0), ("at2", &at.1)],
            )
        };
        out.push(Breaker {
            text,
            slab: at,
            thing: None,
        });
    }
    out
}

// ------------------------------------------------------------------------------- findings

/// Findings of the analysis that are clearly problems, as validation issues (codes start
/// with `analysis-`). Intentional design (asymmetry, uneven keepers, decorative pockets) gives
/// at most a note.
pub fn findings(a: &Analysis) -> Vec<Issue> {
    let mut v = vec![];
    let heart_at = |p: u8| {
        a.sites
            .hearts
            .iter()
            .find(|h| h.owner == p)
            .map(|h| Location::Slab(h.slab.0, h.slab.1))
            .unwrap_or(Location::Map)
    };
    let mut push = |severity: Severity, code: &'static str, at: Location, message: String| {
        v.push(Issue {
            severity,
            code,
            message,
            at,
        })
    };
    // A keeper whose imps reach no portal while the map has portals, or no gold at all. Many
    // campaign levels do this on purpose (creatures and gold from the script, a player walled in
    // against computer keepers), so it is a warning only on a map laid out evenly (at least 90 %
    // of the slabs match one symmetry) where another keeper does have it: an unfair start that
    // cannot be intended. Otherwise a note.
    let even = a.symmetry.iter().any(|c| c.match_fraction >= 0.9);
    let has_portal: Vec<bool> = a
        .economy
        .players
        .iter()
        .map(|pe| pe.portals.iter().any(|d| d.turns != INF))
        .collect();
    let has_gold: Vec<bool> = a
        .economy
        .players
        .iter()
        .map(|pe| pe.reachable_gold_slabs > 0 || pe.reachable_gems > 0 || pe.loose_gold > 0)
        .collect();
    if !a.sites.portals.is_empty() {
        for (k, (pe, f)) in a.economy.players.iter().zip(&a.fields).enumerate() {
            if has_portal[k] {
                continue;
            }
            let crossing = a
                .sites
                .portals
                .iter()
                .any(|po| f.cross.nearest(&po.cells).is_some());
            let unfair = even && has_portal.iter().any(|x| *x);
            if crossing {
                push(
                    Severity::Info,
                    "analysis-portal-across-lava",
                    heart_at(pe.player),
                    trf(
                        "kfx_editor.player_reaches_a_portal_only_across_lava_a_bridg",
                        "{player} reaches a portal only across lava (a bridge or flying \
                        creatures are needed).",
                        &[("player", &owner_name(pe.player))],
                    ),
                );
            } else if unfair {
                push(
                    Severity::Warning,
                    "analysis-no-portal",
                    heart_at(pe.player),
                    trf(
                        "kfx_editor.player_cannot_reach_any_portal_while_other_keepe",
                        "{player} cannot reach any portal while other keepers can: no \
                        creatures will come to this keeper.",
                        &[("player", &owner_name(pe.player))],
                    ),
                );
            } else {
                push(
                    Severity::Info,
                    "analysis-no-portal",
                    heart_at(pe.player),
                    trf(
                        "kfx_editor.player_cannot_reach_any_portal_creatures_must_co",
                        "{player} cannot reach any portal: creatures must come from the \
                        script or a later opening.",
                        &[("player", &owner_name(pe.player))],
                    ),
                );
            }
        }
    }
    for (k, pe) in a.economy.players.iter().enumerate() {
        if has_gold[k] {
            continue;
        }
        if even && has_gold.iter().any(|x| *x) {
            push(
                Severity::Warning,
                "analysis-no-gold",
                heart_at(pe.player),
                trf(
                    "kfx_editor.player_can_reach_no_gold_gems_or_loose_gold_whil",
                    "{player} can reach no gold, gems or loose gold while other keepers can.",
                    &[("player", &owner_name(pe.player))],
                ),
            );
        } else {
            push(
                Severity::Info,
                "analysis-no-gold",
                heart_at(pe.player),
                trf(
                    "kfx_editor.player_can_reach_no_gold_gems_or_loose_gold_mone",
                    "{player} can reach no gold, gems or loose gold: money must come \
                    from the script or a later opening.",
                    &[("player", &owner_name(pe.player))],
                ),
            );
        }
    }
    // hero gates whose heroes reach no keeper at all, even with a tunneller (gates behind lava
    // or rock are a common design, e.g. decoration or scripted spawns: a note)
    if !a.fields.is_empty() {
        for (g, f) in a.sites.gates.iter().zip(&a.gate_fields) {
            let reaches = a
                .fields
                .iter()
                .any(|kf| f.nearest(&a.sites.heart_cells(kf.player)).is_some());
            if !reaches {
                let at = g
                    .thing
                    .map(|t| Location::Item(ItemRef::Thing(t)))
                    .unwrap_or(Location::Slab(g.slab.0, g.slab.1));
                push(
                    Severity::Info,
                    "analysis-gate-isolated",
                    at,
                    trf(
                        "kfx_editor.heroes_from_hero_gate_number_can_never_walk_or_t",
                        "Heroes from hero gate {number} can never walk or tunnel to a \
                        keeper's heart (only flying heroes can).",
                        &[("number", &g.number)],
                    ),
                );
            }
        }
    }
    // keepers that can never meet (note: may be intended, e.g. allies)
    for i in 0..a.fields.len() {
        for j in i + 1..a.fields.len() {
            let (p, q) = (a.fields[i].player, a.fields[j].player);
            let linked = a.routes.iter().any(|r| {
                matches!(
                    (r.from, r.to),
                    (Endpoint::Heart { owner: x }, Endpoint::Heart { owner: y })
                        if (x == p && y == q) || (x == q && y == p)
                )
            });
            let flying = a.fields[i].cross.nearest(&a.sites.heart_cells(q)).is_some();
            if !linked && !flying {
                push(
                    Severity::Info,
                    "analysis-keepers-apart",
                    heart_at(p),
                    trf(
                        "kfx_editor.p_and_q_can_never_reach_each_other_s_hearts",
                        "{p} and {q} can never reach each other's hearts.",
                        &[("p", &owner_name(p)), ("q", &owner_name(q))],
                    ),
                );
            }
        }
    }
    // things in pockets nobody can reach
    for pk in &a.pockets {
        if pk.things.is_empty() {
            continue;
        }
        let at = pk
            .things
            .first()
            .and_then(|k| a.sites.things.get(*k))
            .map(|s| match s.thing {
                Some(t) => Location::Item(ItemRef::Thing(t)),
                None => Location::Slab(s.slab.0, s.slab.1),
            });
        let c = kfx_analysis::sites::group_centre(&pk.cells);
        push(
            Severity::Info,
            "analysis-isolated-things",
            at.unwrap_or(Location::Slab(c.0, c.1)),
            trn(
                "kfx_editor.things_thing_s_in_a_closed_pocket_of_cells_slabs",
                pk.things.len() as i64,
                "{n} thing in a closed pocket of {cells} slabs around ({c},{c2}) \
                that no keeper or hero reaches.",
                "{n} things in a closed pocket of {cells} slabs around ({c},{c2}) \
                that no keeper or hero reaches.",
                &[("cells", &pk.cells.len()), ("c", &c.0), ("c2", &c.1)],
            ),
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfx_analysis::generate::{generate, GenParams};

    fn gd() -> Option<Arc<GameData>> {
        let root = kfx_formats::testutil::install_dir()?;
        Some(Arc::new(
            GameData::load(&crate::document::base_layers(&root)).ok()?,
        ))
    }

    #[test]
    fn parse_names() {
        for k in LayerKind::ALL {
            assert_eq!(LayerKind::parse(k.key()), Some(k));
        }
        for t in AnalysisTab::ALL {
            assert_eq!(AnalysisTab::parse(&t.label().to_lowercase()), Some(t));
        }
        let s: AnalysisSettings = serde_json::from_str("{\"open\": true}").unwrap();
        assert!(s.open && s.validate);
    }

    /// Every layer of a generated 2-keeper map; the worker answers; a generated map has no
    /// warning finding; walling a keeper in gives the "no portal" and "no gold" warnings.
    #[test]
    fn layers_worker_and_findings() {
        let Some(gd) = gd() else { return };
        let g = generate(
            &gd,
            &GenParams {
                seed: 5,
                players: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let a = Analysis::of_doc(&g.doc, &gd);
        for k in LayerKind::ALL {
            let l = build_layer(&a, k, 0, None);
            assert_eq!(
                l.is_none(),
                k == LayerKind::Off || (k == LayerKind::Heroes && a.gate_fields.is_empty()),
                "{k:?}"
            );
            if let Some(l) = l {
                assert_eq!((l.w, l.h), (a.terrain.w, a.terrain.h));
            }
        }
        assert!(
            findings(&a).iter().all(|i| i.severity == Severity::Info),
            "{:?}",
            findings(&a)
        );
        let mut w = Worker::default();
        let key = DocKey::of(&g.doc, 7);
        w.request(&g.doc, gd.clone(), key);
        let d = w
            .wait(key, Duration::from_secs(60))
            .expect("the worker answers");
        assert_eq!(d.key, key);
        assert_eq!(d.analysis.fields.len(), 2);
        // wall the first keeper in with rock
        let mut doc = g.doc.clone();
        let h = a.sites.hearts[0].slab;
        let rock = kfx_map::autotile::kinds::ROCK as u16;
        for y in h.1 - 4..=h.1 + 4 {
            for x in h.0 - 4..=h.0 + 4 {
                if (x - h.0).abs() == 4 || (y - h.1).abs() == 4 {
                    let i = doc.dims.slab_index(x as usize, y as usize);
                    doc.kinds[i] = rock;
                }
            }
        }
        let b = Analysis::of_doc(&doc, &gd);
        let f = findings(&b);
        let codes: Vec<&str> = f.iter().map(|i| i.code).collect();
        assert!(
            codes.contains(&"analysis-no-portal") && codes.contains(&"analysis-no-gold"),
            "{codes:?}"
        );
        // the other keeper has both: an unfair start is a warning
        assert!(
            f.iter()
                .any(|i| i.code == "analysis-no-portal" && i.severity == Severity::Warning),
            "{f:?}"
        );
        assert!(codes.contains(&"analysis-keepers-apart"), "{codes:?}");
        // a breaker list for a symmetric map is empty, a broken one is not
        let m = best_symmetry(&a).unwrap();
        let before = breakers(&a, m).len();
        let after = breakers(&b, m).len();
        assert!(after > before, "{before} -> {after}");
    }

    /// The findings over every shipped map: how often each fires (shipped maps are intended
    /// designs, so warnings must be rare and each one a real oddity).
    #[test]
    fn shipped_findings() {
        use kfx_formats::level_bin;
        use kfx_map::gamedata::{pack_folders, ConfigLayers};
        let Some(root) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let mut counts: std::collections::BTreeMap<(&'static str, String), usize> =
            Default::default();
        let mut warn = vec![];
        let mut n = 0;
        for (levels, cfgs, crtr) in pack_folders(&root) {
            let mut nums: Vec<u32> = std::fs::read_dir(&levels)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .strip_prefix("map")?
                        .strip_suffix(".slb")?
                        .parse()
                        .ok()
                })
                .collect();
            nums.sort();
            nums.dedup();
            for lv in nums {
                let Ok(gd) = GameData::load(&ConfigLayers::for_level(
                    &root,
                    cfgs.as_deref(),
                    Some(&levels),
                    crtr.as_deref(),
                    Some(lv),
                )) else {
                    continue;
                };
                let Some(slb) = level_bin::find_file(&levels, lv, "slb")
                    .and_then(|p| kfx_formats::rnc::load_file(&p).ok())
                else {
                    continue;
                };
                let Some(dims) = level_bin::MapDims::from_slb_len(slb.len()) else {
                    continue;
                };
                let Ok(doc) = MapDoc::load(&levels, lv, dims, &gd) else {
                    continue;
                };
                n += 1;
                let a = Analysis::of_doc(&doc, &gd);
                for i in findings(&a) {
                    *counts
                        .entry((i.code, format!("{:?}", i.severity)))
                        .or_default() += 1;
                    if i.severity == Severity::Warning {
                        warn.push(format!(
                            "{}/map{lv:05}: {}",
                            levels.file_name().unwrap_or_default().to_string_lossy(),
                            i.message
                        ));
                    }
                }
            }
        }
        println!("{n} shipped maps; findings: {counts:?}");
        for w in &warn {
            println!("  warning {w}");
        }
        assert!(
            warn.is_empty(),
            "shipped maps are intended designs: no analysis warning expected"
        );
    }
}
