//! The New map dialog's maps. Two ways:
//! * fill: one slab kind (with a rock border), keepers with a dungeon heart in a small claimed
//!   area, a portal and gold nearby, imps, hero gates. [`plan`] gives the slab layout and the
//!   items (fast, for the dialog's live preview); [`build`] makes the map from a plan.
//! * generate: the constrained generator of `kfx_analysis::generate` (styles, symmetry, keeper
//!   constraints checked with the map analyses, seeded and deterministic). [`gen_params`] turns
//!   the spec into its parameters, [`build_generated`] makes the document, and the starter
//!   script comes from the generator's [`StarterScript`] ([`dk_starter`], [`lua_starter`]).
use crate::assets::slab_color;
use crate::document::{apply_edits, Document, Edit};
use eframe::egui::{Color32, ColorImage};
use kfx_analysis::generate::{GenParams, Generated, StarterScript, Style, SymmetryChoice};
use kfx_formats::level_bin::MapDims;
use kfx_map::autotile::{kinds, PLAYER_NEUTRAL};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things as kthings;
use kfx_map::transform::SymmetryMode;
use kfx_prefab::i18n::{trd, trf, tx, Tx};
use std::path::Path;
use std::sync::Arc;

/// Keepers in placement order: red, blue, green, yellow, purple, black, orange (owner 4 is the
/// heroes, 5 neutral).
pub const KEEPERS: [u8; 7] = [0, 1, 2, 3, 6, 7, 8];

/// Script names of the owners (`player_desc`, data/script_tables.json).
pub fn player_name(o: u8) -> &'static str {
    match o {
        0 => "PLAYER0",
        1 => "PLAYER1",
        2 => "PLAYER2",
        3 => "PLAYER3",
        4 => "PLAYER_GOOD",
        6 => "PLAYER4",
        7 => "PLAYER5",
        8 => "PLAYER6",
        _ => "PLAYER_NEUTRAL",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptChoice {
    None,
    Dk,
    Lua,
}

/// Everything the New map dialog sets.
#[derive(Clone, Debug, PartialEq)]
pub struct NewMapSpec {
    pub w: u32,
    pub h: u32,
    /// Impenetrable rock on the outer ring.
    pub border_rock: bool,
    /// Slab kind name filling the map when no terrain is generated (`EARTH`, `ROCK`, `PATH`...).
    pub fill: String,
    pub texture: u8,
    /// Use the constrained generator (`kfx_analysis::generate`) instead of the fill.
    pub generate: bool,
    pub seed: u64,
    /// Generator options (when `generate`).
    pub gen: GenOptions,
    /// Symmetry of the keeper placement of a filled map.
    pub symmetry: Option<SymmetryMode>,
    /// Keepers (0..=7) with a heart each.
    pub players: u8,
    pub imps: u8,
    pub portals: bool,
    pub gold_near: bool,
    pub hero_gates: u8,
    pub script: ScriptChoice,
    /// The level's kind (`.lof` KIND, written with the first save; `None` = no `.lof` kind).
    pub kind: Option<kfx_formats::levelinfo::LevelKind>,
    /// The level's name (`.lof` NAME_TEXT, written with the first save).
    pub name: String,
}

impl Default for NewMapSpec {
    fn default() -> Self {
        NewMapSpec {
            w: 85,
            h: 85,
            border_rock: true,
            fill: "EARTH".into(),
            texture: 0,
            generate: false,
            seed: 1,
            gen: GenOptions::default(),
            symmetry: None,
            players: 0,
            imps: 4,
            portals: true,
            gold_near: true,
            hero_gates: 0,
            script: ScriptChoice::None,
            kind: None,
            name: String::new(),
        }
    }
}

/// Options of the constrained generator (the rest of its parameters come from the spec: size,
/// seed, keepers, imps, hero gates).
#[derive(Clone, Debug, PartialEq)]
pub struct GenOptions {
    pub style: Style,
    pub symmetry: SymmetryChoice,
    /// Gold amount 0..=100.
    pub richness: u8,
    /// Water and lava amounts 0..=100.
    pub water: u8,
    pub lava: u8,
    /// Hero gates (and the fortress garrison).
    pub heroes: bool,
    /// Gem clusters.
    pub gems: u8,
    /// Neutral chambers, creatures in them, special boxes in them.
    pub neutral_rooms: u8,
    pub neutral_creatures: u8,
    pub special_boxes: u8,
    /// Minimum gold per keeper (0 = from the richness).
    pub min_gold: u64,
    /// Largest fairness imbalance accepted (0 = perfectly even).
    pub max_imbalance: f32,
    pub max_attempts: u32,
}

impl Default for GenOptions {
    fn default() -> Self {
        let p = GenParams::default();
        GenOptions {
            style: p.style,
            symmetry: p.symmetry,
            richness: p.richness,
            water: p.water,
            lava: p.lava,
            heroes: p.heroes,
            gems: p.gems,
            neutral_rooms: p.neutral_rooms,
            neutral_creatures: p.neutral_creatures,
            special_boxes: p.special_boxes,
            min_gold: p.min_gold,
            max_imbalance: p.max_imbalance,
            max_attempts: p.max_attempts,
        }
    }
}

/// Name of a generator style (dialog, `--actions`).
pub fn style_key(s: Style) -> &'static str {
    match s {
        Style::OpenCave => "open_cave",
        Style::Chambers => "chambers",
        Style::Islands => "islands",
        Style::River => "river",
        Style::Fortress => "fortress",
    }
}

pub fn style_label(s: Style) -> Tx {
    match s {
        Style::OpenCave => tx("kfx_editor.open_cave", "Open cave"),
        Style::Chambers => tx("kfx_editor.chambers_c676", "Chambers"),
        Style::Islands => tx("kfx_editor.islands", "Islands"),
        Style::River => tx("kfx_editor.river", "River"),
        Style::Fortress => tx("kfx_editor.fortress", "Fortress"),
    }
}

pub fn style_tooltip(s: Style) -> Tx {
    match s {
        Style::OpenCave => tx(
            "kfx_editor.earth_with_rock_outcrops_gold_veins_pools_and_op",
            "Earth with rock outcrops, gold veins, pools and open caverns.",
        ),
        Style::Chambers => tx(
            "kfx_editor.rock_heavy_with_pre_dug_chambers_joined_by_corri",
            "Rock-heavy, with pre-dug chambers joined by corridors.",
        ),
        Style::Islands => tx(
            "kfx_editor.keepers_on_islands_in_a_lava_or_water_sea_joined",
            "Keepers on islands in a lava or water sea, joined by earth causeways.",
        ),
        Style::River => tx(
            "kfx_editor.rivers_of_lava_or_water_between_the_keepers_with",
            "Rivers of lava or water between the keepers, with fords.",
        ),
        Style::Fortress => tx(
            "kfx_editor.a_walled_hero_stronghold_in_the_centre",
            "A walled hero stronghold in the centre.",
        ),
    }
}

pub fn parse_style(s: &str) -> Option<Style> {
    kfx_analysis::generate::ALL_STYLES
        .iter()
        .copied()
        .find(|x| {
            style_key(*x).eq_ignore_ascii_case(s.trim())
                || style_label(*x).en.eq_ignore_ascii_case(s.trim())
        })
}

/// Name of a generator symmetry choice (`--actions`: `auto`, `none`, `radial`, `ns`, `ew`,
/// `fourway`, `rot2`, `rot4`).
pub fn parse_gen_symmetry(s: &str) -> Option<SymmetryChoice> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "auto" => SymmetryChoice::Auto,
        "none" => SymmetryChoice::None,
        "radial" => SymmetryChoice::Radial,
        other => SymmetryChoice::Mode(parse_symmetry(other)?),
    })
}

/// `ns`, `ew`, `fourway`, `rot2`, `rot4`.
pub fn parse_symmetry(s: &str) -> Option<SymmetryMode> {
    Some(match s {
        "ns" => SymmetryMode::NorthSouth,
        "ew" => SymmetryMode::EastWest,
        "fourway" => SymmetryMode::FourWay,
        "rot2" => SymmetryMode::Rot2,
        "rot4" => SymmetryMode::Rot4,
        _ => return None,
    })
}

/// The generator parameters of a spec.
pub fn gen_params(spec: &NewMapSpec, folder: &Path, level: u32, decorate: bool) -> GenParams {
    let g = &spec.gen;
    GenParams {
        seed: spec.seed,
        width: spec.w.clamp(20, 170) as u16,
        height: spec.h.clamp(20, 170) as u16,
        style: g.style,
        players: spec.players.min(KEEPERS.len() as u8),
        symmetry: g.symmetry,
        richness: g.richness.min(100),
        water: g.water.min(100),
        lava: g.lava.min(100),
        heroes: g.heroes,
        hero_gates: spec.hero_gates,
        gems: g.gems,
        neutral_rooms: g.neutral_rooms,
        neutral_creatures: g.neutral_creatures,
        special_boxes: g.special_boxes,
        imps: spec.imps,
        min_gold: g.min_gold,
        max_imbalance: g.max_imbalance,
        max_attempts: g.max_attempts.clamp(1, 64),
        decorate,
        folder: folder.to_path_buf(),
        level,
    }
}

/// Apply one generator key of a JSON spec (`--actions` `new_map` / `new_dialog`); `Ok(false)`
/// when the key is not a generator key.
pub fn apply_gen_key(
    spec: &mut NewMapSpec,
    k: &str,
    val: &serde_json::Value,
) -> Result<bool, String> {
    let u = || val.as_u64().unwrap_or(0);
    let pct = || -> u8 {
        // 0..100, or a share 0..1 as the noise terrain took
        match val.as_f64() {
            Some(f) if f > 0.0 && f <= 1.0 && !val.is_u64() => (f * 100.0).round() as u8,
            Some(f) => f.clamp(0.0, 100.0) as u8,
            None => 0,
        }
    };
    let g = &mut spec.gen;
    match k {
        "style" => {
            g.style = parse_style(val.as_str().unwrap_or("")).ok_or_else(|| {
                format!("unknown style {val} (open_cave, chambers, islands, river, fortress)")
            })?
        }
        "gen_symmetry" => {
            g.symmetry = parse_gen_symmetry(val.as_str().unwrap_or("")).ok_or_else(|| {
                format!(
                    "unknown generator symmetry {val} (auto, none, radial, ns, ew, fourway, \
                    rot2, rot4)"
                )
            })?
        }
        "richness" | "gold" => g.richness = pct(),
        "water" => g.water = pct(),
        "lava" => g.lava = pct(),
        "heroes" => g.heroes = val.as_bool().unwrap_or(true),
        "gems" => g.gems = u() as u8,
        "neutral_rooms" | "chambers" => g.neutral_rooms = u() as u8,
        "neutral_creatures" => g.neutral_creatures = u() as u8,
        "special_boxes" => g.special_boxes = u() as u8,
        "min_gold" => g.min_gold = u(),
        "max_imbalance" => g.max_imbalance = val.as_f64().unwrap_or(0.25) as f32,
        "max_attempts" | "attempts" => g.max_attempts = u().clamp(1, 64) as u32,
        // accepted and ignored: settings of an older noise terrain generator
        "scale" | "earth" | "rock" => {}
        _ => return Ok(false),
    }
    Ok(true)
}

/// The document of a generated map (no undo history, not saved yet).
pub fn build_generated(
    g: &Generated,
    spec: &NewMapSpec,
    folder: &Path,
    level: u32,
    gd: Arc<GameData>,
    stack: kfx_map::mods::LayerStack,
    decorate: bool,
) -> Document {
    let mut doc = g.doc.clone();
    doc.folder = folder.to_path_buf();
    doc.level = level;
    doc.set_texture_id(spec.texture);
    let mut out = Document::new(doc, gd, stack, None, false);
    out.decorator.enabled = decorate;
    out
}

/// The top-down preview of a generated map (terrain, routes, hearts, gates, portals) from
/// `kfx_analysis`'s renderer.
pub fn generated_preview(a: &kfx_analysis::Analysis) -> ColorImage {
    let scale = (512 / a.terrain.w.max(a.terrain.h).max(1)).clamp(2, 8) as u32;
    let img = kfx_analysis::layer::render_overview(a, scale);
    ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.pixels)
}

/// The DK level script of the generator's starter values: version, speed, money, computer
/// keepers, creature pool, rooms, powers, win / lose conditions.
pub fn dk_starter(level: u32, s: &StarterScript) -> String {
    let mut out = format!(
        "REM map{level:05}.txt: generated by the KeeperFX map \
        editor\nLEVEL_VERSION(1)\nSET_GENERATE_SPEED({})\n\n",
        s.generate_speed
    );
    for p in &s.players {
        out += &format!(
            "START_MONEY({},{})\nMAX_CREATURES({},{})\n",
            p.script_name, p.start_money, p.script_name, p.max_creatures
        );
        if let Some(c) = p.computer {
            out += &format!("COMPUTER_PLAYER({},{c})\n", p.script_name);
        }
    }
    out.push('\n');
    for (c, n) in &s.creature_pool {
        out += &format!("ADD_CREATURE_TO_POOL({c},{n})\nCREATURE_AVAILABLE(ALL_PLAYERS,{c},1,0)\n");
    }
    out.push('\n');
    for (r, now) in &s.rooms {
        out += &format!("ROOM_AVAILABLE(ALL_PLAYERS,{r},1,{})\n", *now as u8);
    }
    for (p, now) in &s.powers {
        out += &format!("MAGIC_AVAILABLE(ALL_PLAYERS,{p},1,{})\n", *now as u8);
    }
    out.push('\n');
    if let Some(first) = s.players.first() {
        if s.players.len() > 1 {
            out += &format!(
                "IF({},ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\nENDIF\n",
                first.script_name
            );
        }
        out += &format!(
            "IF({},DUNGEON_DESTROYED == 1)\n\tLOSE_GAME\nENDIF\n",
            first.script_name
        );
    }
    out
}

/// The Lua version of [`dk_starter`] (the stock `bindings/setup.lua` functions).
pub fn lua_starter(level: u32, s: &StarterScript) -> String {
    let mut out = format!(
        "-- map{level:05}.lua: generated by the KeeperFX map \
        editor\n\n---Called once when the level starts.\nfunction OnGameStart()\n    \
        SetGenerateSpeed({})\n",
        s.generate_speed
    );
    for p in &s.players {
        out += &format!(
            "    StartMoney({}, {})\n    MaxCreatures({}, {})\n",
            p.script_name, p.start_money, p.script_name, p.max_creatures
        );
        if let Some(c) = p.computer {
            out += &format!("    ComputerPlayer({}, {c})\n", p.script_name);
        }
    }
    for (c, n) in &s.creature_pool {
        out += &format!(
            "    AddCreatureToPool(\"{c}\", {n})\n    CreatureAvailable(ALL_PLAYERS, \
            \"{c}\", true, 0)\n"
        );
    }
    for (r, now) in &s.rooms {
        out += &format!("    RoomAvailable(ALL_PLAYERS, \"{r}\", true, {now})\n");
    }
    for (p, now) in &s.powers {
        out += &format!("    MagicAvailable(ALL_PLAYERS, \"{p}\", true, {now})\n");
    }
    out += "end\n";
    out
}

/// A keeper's base.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Base {
    pub owner: u8,
    /// Heart centre slab.
    pub heart: (i32, i32),
    /// Portal centre slab.
    pub portal: Option<(i32, i32)>,
}

/// The planned map: slab kinds and owners, bases, hero gates (number, slab).
#[derive(Clone, Debug)]
pub struct Plan {
    pub w: usize,
    pub h: usize,
    pub kinds: Vec<u16>,
    pub owners: Vec<u8>,
    pub bases: Vec<Base>,
    pub gates: Vec<(i64, (i32, i32))>,
    pub warnings: Vec<String>,
}

// ------------------------------------------------------------------------------ symmetry

/// The images of slab (x, y) under a symmetry (itself first).
pub fn orbit(mode: Option<SymmetryMode>, w: i32, h: i32, x: i32, y: i32) -> Vec<(i32, i32)> {
    let (mx, my) = (w - 1 - x, h - 1 - y);
    let mut v = match mode {
        None => vec![(x, y)],
        Some(SymmetryMode::NorthSouth) => vec![(x, y), (x, my)],
        Some(SymmetryMode::EastWest) => vec![(x, y), (mx, y)],
        Some(SymmetryMode::FourWay) => vec![(x, y), (mx, y), (x, my), (mx, my)],
        Some(SymmetryMode::Rot2) => vec![(x, y), (mx, my)],
        // quarter turns need a square map; the caller makes it square
        Some(SymmetryMode::Rot4) => vec![(x, y), (w - 1 - y, x), (mx, my), (y, h - 1 - x)],
    };
    let first = v[0];
    v.dedup();
    if v.is_empty() {
        v.push(first);
    }
    v
}

// ------------------------------------------------------------------------------ plan

fn room_slab(gd: &GameData, room: &str) -> Option<u16> {
    gd.room_id(room)
        .map(|r| gd.rooms[r].assigned_slab as u16)
        .filter(|&k| k > 0)
}

/// Points spread over the fundamental region of a symmetry (the part every orbit has one point
/// in), `k` of them, at a "radius" `r` (0..1) from the map centre.
fn region_seeds(mode: SymmetryMode, w: i32, h: i32, k: usize, r: f32) -> Vec<(i32, i32)> {
    let (cx, cy) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    // angular range of the region (screen angles, y down), keepers spread inside it
    let (a0, a1) = match mode {
        SymmetryMode::NorthSouth | SymmetryMode::Rot2 => {
            (std::f32::consts::PI, std::f32::consts::TAU)
        }
        SymmetryMode::EastWest => (std::f32::consts::FRAC_PI_2, 1.5 * std::f32::consts::PI),
        SymmetryMode::FourWay | SymmetryMode::Rot4 => {
            (std::f32::consts::PI, 1.5 * std::f32::consts::PI)
        }
    };
    (0..k)
        .map(|i| {
            let a = a0 + (a1 - a0) * (i as f32 + 0.5) / k as f32;
            (
                (cx + r * w as f32 / 2.0 * a.cos()).round() as i32,
                (cy + r * h as f32 / 2.0 * a.sin()).round() as i32,
            )
        })
        .collect()
}

/// `n` positions around the centre at radius `r` (0..1): symmetric images of seeds when the
/// symmetry allows `n` (a multiple of its orbit size, or 1 = the centre), else evenly spaced on
/// an ellipse (then the warning says the layout is not symmetric).
fn ring_positions(
    mode: Option<SymmetryMode>,
    w: i32,
    h: i32,
    n: usize,
    r: f32,
    phase: f32,
    warn: &mut Vec<String>,
    what: &str,
) -> Vec<(i32, i32)> {
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![(w / 2, h / 2)];
    }
    if let Some(m) = mode {
        let orbit_len = orbit(Some(m), w, h, w / 4, h / 4).len();
        if n % orbit_len == 0 {
            let mut out = vec![];
            for s in region_seeds(m, w, h, n / orbit_len, r) {
                out.extend(orbit(Some(m), w, h, s.0, s.1));
            }
            return out;
        }
        warn.push(trf(
            "kfx_editor.n_what_cannot_be_placed_symmetrically_use_a_mult",
            "{n} {what} cannot be placed symmetrically (use a multiple of \
            {orbit_len}): placed on a ring",
            &[("n", &n), ("what", &what), ("orbit_len", &orbit_len)],
        ));
    }
    let (cx, cy) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    (0..n)
        .map(|i| {
            let a = phase + i as f32 * std::f32::consts::TAU / n as f32;
            (
                (cx + r * w as f32 / 2.0 * a.cos()).round() as i32,
                (cy + r * h as f32 / 2.0 * a.sin()).round() as i32,
            )
        })
        .collect()
}

/// Plan a new map (see the module docs).
pub fn plan(spec: &NewMapSpec, gd: &GameData) -> Plan {
    let mut warnings = vec![];
    let (mut w, mut h) = (spec.w.clamp(1, 170) as i32, spec.h.clamp(1, 170) as i32);
    if spec.symmetry == Some(SymmetryMode::Rot4) && w != h {
        let s = w.min(h);
        warnings.push(trf(
            "kfx_editor.rotational_4_symmetry_needs_a_square_map_s_x_s",
            "rotational 4 symmetry needs a square map: {s}x{s}",
            &[("s", &s)],
        ));
        w = s;
        h = s;
    }
    let n = (w * h) as usize;
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let fill = gd
        .slab_id(&spec.fill)
        .map(|k| k as u16)
        .unwrap_or(kinds::EARTH as u16);
    let mut kinds_v = vec![fill; n];
    let mut owners = vec![PLAYER_NEUTRAL; n];
    let (rock, gold) = (kinds::ROCK as u16, kinds::GOLD as u16);
    // bases
    let heart = room_slab(gd, "DUNGEON_HEART");
    let portal = room_slab(gd, "ENTRANCE");
    if spec.players > 0 && heart.is_none() {
        warnings.push(
            trd(
                "kfx_editor.no_dungeon_heart_room_in_terrain_cfg_no_hearts_p",
                "no DUNGEON_HEART room in terrain.cfg: no hearts placed",
            )
            .into(),
        );
    }
    let claimed = kinds::CLAIMED as u16;
    let path = kinds::PATH as u16;
    let np = (spec.players as usize).min(KEEPERS.len());
    let positions = ring_positions(
        spec.symmetry,
        w,
        h,
        np,
        0.62,
        -std::f32::consts::FRAC_PI_4 * 3.0,
        &mut warnings,
        "keepers",
    );
    let (cx, cy) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    let inside = |x: i32, y: i32| x >= 1 && y >= 1 && x < w - 1 && y < h - 1;
    let set = |kinds_v: &mut Vec<u16>, owners: &mut Vec<u8>, x: i32, y: i32, k: u16, o: u8| {
        if inside(x, y) {
            kinds_v[idx(x, y)] = k;
            owners[idx(x, y)] = o;
        }
    };
    let mut bases = vec![];
    for (i, &(hx, hy)) in positions.iter().enumerate() {
        let owner = KEEPERS[i];
        let Some(hk) = heart else { break };
        // 9x9 claimed area with the 3x3 heart in the middle
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let k = if dx.abs() <= 1 && dy.abs() <= 1 {
                    hk
                } else {
                    claimed
                };
                set(&mut kinds_v, &mut owners, hx + dx, hy + dy, k, owner);
            }
        }
        // towards the map centre (portal) and away from it (gold)
        let (vx, vy) = (cx - hx as f32, cy - hy as f32);
        let len = (vx * vx + vy * vy).sqrt();
        let (ux, uy) = if len > 0.5 {
            (vx / len, vy / len)
        } else {
            (1.0, 0.0)
        };
        let mut base = Base {
            owner,
            heart: (hx, hy),
            portal: None,
        };
        if spec.portals {
            if let Some(pk) = portal {
                let (px, py) = (
                    (hx as f32 + ux * 9.0).round() as i32,
                    (hy as f32 + uy * 9.0).round() as i32,
                );
                if inside(px - 1, py - 1) && inside(px + 1, py + 1) {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            set(
                                &mut kinds_v,
                                &mut owners,
                                px + dx,
                                py + dy,
                                pk,
                                PLAYER_NEUTRAL,
                            );
                        }
                    }
                    base.portal = Some((px, py));
                }
            }
        }
        if spec.gold_near {
            let (gx, gy) = (
                (hx as f32 - ux * 7.0).round() as i32,
                (hy as f32 - uy * 7.0).round() as i32,
            );
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if inside(gx + dx, gy + dy) && (dx + dy) % 2 == 0 {
                        set(
                            &mut kinds_v,
                            &mut owners,
                            gx + dx,
                            gy + dy,
                            gold,
                            PLAYER_NEUTRAL,
                        );
                    }
                }
            }
        }
        bases.push(base);
    }
    // hero gates: on a smaller ring between the bases (or the centre for one)
    let mut gates = vec![];
    let ng = spec.hero_gates as usize;
    let gate_pos = if ng == 1 && np != 1 {
        vec![(w / 2, h / 2)]
    } else {
        ring_positions(
            spec.symmetry,
            w,
            h,
            ng,
            0.3,
            -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI / ng.max(1) as f32,
            &mut warnings,
            "hero gates",
        )
    };
    for (g, &(gx, gy)) in gate_pos.iter().enumerate() {
        for dy in -1..=1 {
            for dx in -1..=1 {
                set(
                    &mut kinds_v,
                    &mut owners,
                    gx + dx,
                    gy + dy,
                    path,
                    PLAYER_NEUTRAL,
                );
            }
        }
        if inside(gx, gy) {
            gates.push((g as i64 + 1, (gx, gy)));
        }
    }
    if spec.border_rock {
        for y in 0..h {
            for x in 0..w {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    kinds_v[idx(x, y)] = rock;
                    owners[idx(x, y)] = PLAYER_NEUTRAL;
                }
            }
        }
    }
    Plan {
        w: w as usize,
        h: h as usize,
        kinds: kinds_v,
        owners,
        bases,
        gates,
        warnings,
    }
}

/// The preview of a plan: slab colours, the hearts and portals outlined in their owner colour.
pub fn preview(p: &Plan, gd: &GameData) -> ColorImage {
    let mut px: Vec<Color32> = (0..p.w * p.h)
        .map(|i| {
            let c = slab_color(gd, p.kinds[i], p.owners[i]);
            Color32::from_rgb(c[0], c[1], c[2])
        })
        .collect();
    for (_, (x, y)) in &p.gates {
        let i = *y as usize * p.w + *x as usize;
        if i < px.len() {
            px[i] = Color32::WHITE;
        }
    }
    ColorImage::new([p.w, p.h], px)
}

/// Make the map of a plan (no undo history): slabs placed like the editor's tools (rock style,
/// torch dirt, columns, decoration, slab objects such as the heart and torches), imps around
/// each heart and the hero gates.
#[allow(clippy::too_many_arguments)]
pub fn build(
    p: &Plan,
    spec: &NewMapSpec,
    folder: &Path,
    level: u32,
    gd: Arc<GameData>,
    stack: kfx_map::mods::LayerStack,
    decorate: bool,
) -> Document {
    let dims = MapDims::new(p.w as u16, p.h as u16);
    let base = Document::create(
        folder,
        level,
        dims,
        kinds::EARTH as u16,
        false,
        spec.texture,
        gd.clone(),
        stack,
    );
    let mut d = base;
    d.decorator.enabled = decorate;
    let mut edits = vec![];
    for y in 0..p.h {
        for x in 0..p.w {
            let i = y * p.w + x;
            edits.push((
                x as i32,
                y as i32,
                Edit::Place {
                    kind: p.kinds[i],
                    owner: p.owners[i],
                },
            ));
        }
    }
    let dec = d.decorator.clone();
    apply_edits(&mut d.doc, &gd, &dec, &edits);
    add_items(&mut d.doc, &gd, p, spec);
    let stack = d.stack.clone();
    let mut out = Document::new(d.doc, gd, stack, None, false);
    out.decorator.enabled = decorate;
    out
}

fn add_items(m: &mut MapDoc, gd: &GameData, p: &Plan, spec: &NewMapSpec) {
    use kfx_formats::level_fx::StlCoord;
    let imp = gd.creature_id("IMP").map(|i| i as i64);
    for b in &p.bases {
        let (hx, hy) = b.heart;
        let spots = [
            (-3, 0),
            (3, 0),
            (0, -3),
            (0, 3),
            (-3, -3),
            (3, 3),
            (-3, 3),
            (3, -3),
            (-2, -3),
            (2, 3),
        ];
        if let Some(imp) = imp {
            for &(dx, dy) in spots.iter().take(spec.imps as usize) {
                let (sx, sy) = (3 * (hx + dx) + 1, 3 * (hy + dy) + 1);
                let mut t = kthings::new_thing(
                    kthings::class::CREATURE,
                    imp,
                    b.owner,
                    kthings::centred(sx),
                    kthings::centred(sy),
                    m.floor_z(sx, sy),
                );
                t.creature_level = Some(1);
                let _ = m.add_thing(gd, t);
            }
        }
    }
    if let Some(model) = kthings::hero_gate_model(gd) {
        for &(n, (x, y)) in &p.gates {
            let (sx, sy) = (3 * x + 1, 3 * y + 1);
            let mut t = kthings::new_thing(
                kthings::class::OBJECT,
                model,
                4,
                kthings::centred(sx),
                kthings::centred(sy),
                m.floor_z(sx, sy),
            );
            t.herogate_number = Some(n);
            let _ = m.add_thing(gd, t);
        }
    }
    let _ = StlCoord::new(0, 0);
}

/// A playable DK script for the keepers of a plan: level version, generation speed, money,
/// computer keepers for every keeper but the first, creature pool, rooms and powers, win / lose
/// conditions.
pub fn dk_script(level: u32, p: &Plan, gd: &GameData) -> String {
    let mut s = format!(
        "REM map{level:05}.txt: generated by the KeeperFX map \
        editor\nLEVEL_VERSION(1)\nSET_GENERATE_SPEED(500)\n\n"
    );
    let owners: Vec<u8> = p.bases.iter().map(|b| b.owner).collect();
    for (i, o) in owners.iter().enumerate() {
        s.push_str(&format!(
            "START_MONEY({},5000)\nMAX_CREATURES({},30)\n",
            player_name(*o),
            player_name(*o)
        ));
        if i > 0 {
            s.push_str(&format!("COMPUTER_PLAYER({},0)\n", player_name(*o)));
        }
    }
    s.push('\n');
    let has = |n: &str| gd.creature_id(n).is_some();
    for c in [
        "FLY",
        "BUG",
        "SPIDER",
        "DEMONSPAWN",
        "TROLL",
        "ORC",
        "SORCEROR",
        "DRAGON",
    ] {
        if has(c) {
            s.push_str(&format!(
                "ADD_CREATURE_TO_POOL({c},10)\nCREATURE_AVAILABLE(ALL_PLAYERS,{c},1,0)\n"
            ));
        }
    }
    s.push('\n');
    for r in [
        "TREASURE",
        "LAIR",
        "GARDEN",
        "TRAINING",
        "RESEARCH",
        "WORKSHOP",
        "BARRACKS",
        "LIBRARY",
        "BRIDGE",
        "GUARD_POST",
        "PRISON",
        "TORTURE",
    ] {
        if gd.room_id(r).is_some() {
            let now = matches!(r, "TREASURE" | "LAIR" | "GARDEN" | "TRAINING" | "RESEARCH");
            s.push_str(&format!(
                "ROOM_AVAILABLE(ALL_PLAYERS,{r},1,{})\n",
                now as u8
            ));
        }
    }
    s.push_str(
        "MAGIC_AVAILABLE(ALL_PLAYERS,POWER_IMP,1,1)\n\
        MAGIC_AVAILABLE(ALL_PLAYERS,POWER_SLAP,1,1)\n\
        MAGIC_AVAILABLE(ALL_PLAYERS,POWER_SIGHT,1,0)\n\
        MAGIC_AVAILABLE(ALL_PLAYERS,POWER_CALL_TO_ARMS,1,0)\n\n",
    );
    if owners.len() > 1 {
        s.push_str("IF(PLAYER0,ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\nENDIF\n");
    }
    if !owners.is_empty() {
        s.push_str("IF(PLAYER0,DUNGEON_DESTROYED == 1)\n\tLOSE_GAME\nENDIF\n");
    }
    s
}

/// The Lua version of [`dk_script`] (the stock `bindings/setup.lua` functions).
pub fn lua_script(level: u32, p: &Plan, gd: &GameData) -> String {
    let mut s = format!(
        "-- map{level:05}.lua: generated by the KeeperFX map editor\n\n---Called \
        once when the level starts.\nfunction OnGameStart()\n    SetGenerateSpeed(500)\n"
    );
    for (i, b) in p.bases.iter().enumerate() {
        let pl = player_name(b.owner);
        s.push_str(&format!(
            "    StartMoney({pl}, 5000)\n    MaxCreatures({pl}, 30)\n"
        ));
        if i > 0 {
            s.push_str(&format!("    ComputerPlayer({pl}, 0)\n"));
        }
    }
    for c in [
        "FLY",
        "BUG",
        "SPIDER",
        "DEMONSPAWN",
        "TROLL",
        "ORC",
        "SORCEROR",
        "DRAGON",
    ] {
        if gd.creature_id(c).is_some() {
            s.push_str(&format!(
                "    AddCreatureToPool(\"{c}\", 10)\n    \
                CreatureAvailable(ALL_PLAYERS, \"{c}\", true, 0)\n"
            ));
        }
    }
    for r in [
        "TREASURE",
        "LAIR",
        "GARDEN",
        "TRAINING",
        "RESEARCH",
        "WORKSHOP",
        "BARRACKS",
        "LIBRARY",
        "BRIDGE",
        "GUARD_POST",
        "PRISON",
        "TORTURE",
    ] {
        if gd.room_id(r).is_some() {
            let now = matches!(r, "TREASURE" | "LAIR" | "GARDEN" | "TRAINING" | "RESEARCH");
            s.push_str(&format!(
                "    RoomAvailable(ALL_PLAYERS, \"{r}\", true, {now})\n"
            ));
        }
    }
    s.push_str(
        "    MagicAvailable(ALL_PLAYERS, \"POWER_IMP\", true, true)\n    \
        MagicAvailable(ALL_PLAYERS, \"POWER_SLAP\", true, true)\nend\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gd() -> Option<(Arc<GameData>, kfx_map::mods::LayerStack)> {
        let root = kfx_formats::testutil::install_dir()?;
        let layers = crate::document::base_layers(&root);
        Some((
            Arc::new(GameData::load(&layers).ok()?),
            kfx_map::mods::stack_of_layers(&layers),
        ))
    }

    #[test]
    fn orbits() {
        assert_eq!(
            orbit(Some(SymmetryMode::FourWay), 10, 8, 1, 2),
            vec![(1, 2), (8, 2), (1, 5), (8, 5)]
        );
        assert_eq!(
            orbit(Some(SymmetryMode::Rot4), 10, 10, 1, 2),
            vec![(1, 2), (7, 1), (8, 7), (2, 8)]
        );
    }

    /// The generator path: a spec with generator options gives a valid document with every
    /// keeper, the starter scripts name every keeper, the JSON keys parse.
    #[test]
    fn generated_map_and_starter_scripts() {
        let Some((gd, layers)) = gd() else { return };
        let mut spec = NewMapSpec {
            generate: true,
            seed: 11,
            players: 3,
            imps: 3,
            texture: 2,
            ..Default::default()
        };
        for (k, v) in [
            ("style", serde_json::json!("chambers")),
            ("gen_symmetry", serde_json::json!("radial")),
            ("richness", serde_json::json!(70)),
            ("water", serde_json::json!(0.1)),
            ("max_attempts", serde_json::json!(12)),
        ] {
            assert!(apply_gen_key(&mut spec, k, &v).unwrap(), "{k}");
        }
        assert!(!apply_gen_key(&mut spec, "w", &serde_json::json!(5)).unwrap());
        assert!(apply_gen_key(&mut spec, "style", &serde_json::json!("nope")).is_err());
        assert_eq!(
            (
                spec.gen.style,
                spec.gen.symmetry,
                spec.gen.richness,
                spec.gen.water
            ),
            (Style::Chambers, SymmetryChoice::Radial, 70, 10)
        );
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/newmap_gen_{}",
            std::process::id()
        ));
        let p = gen_params(&spec, &dir, 7, true);
        assert_eq!((p.players, p.imps, p.seed, p.level), (3, 3, 11, 7));
        let g = kfx_analysis::generate::generate(&gd, &p).unwrap_or_else(|e| panic!("{e}"));
        let d = build_generated(&g, &spec, &dir, 7, gd.clone(), layers, true);
        assert_eq!(d.doc.texture_id, 2);
        assert!(d.is_dirty());
        let errors: Vec<_> = kfx_map::validate::validate(&d.doc, &gd)
            .into_iter()
            .filter(|i| i.severity == kfx_map::validate::Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let a = kfx_analysis::Analysis::of_doc(&d.doc, &gd);
        assert_eq!(a.fields.len(), 3);
        let img = generated_preview(&a);
        assert!(img.size[0] >= 170 && img.size[0] == img.size[1]);
        let s = dk_starter(7, &g.script);
        assert!(
            s.contains("COMPUTER_PLAYER(PLAYER2,0)")
                && !s.contains("COMPUTER_PLAYER(PLAYER0")
                && s.contains("WIN_GAME"),
            "{s}"
        );
        let l = lua_starter(7, &g.script);
        assert!(
            l.contains("ComputerPlayer(PLAYER1, 0)")
                && l.contains("MagicAvailable(ALL_PLAYERS, \"POWER_IMP\", true, true)"),
            "{l}"
        );
    }

    /// A 4-keeper map: hearts with their heart objects (from the slab objects), imps, portals,
    /// gates; the map validates without errors and has the right owners.
    #[test]
    fn build_four_keepers() {
        let Some((gd, layers)) = gd() else { return };
        let spec = NewMapSpec {
            players: 4,
            imps: 4,
            hero_gates: 2,
            symmetry: Some(SymmetryMode::FourWay),
            seed: 3,
            ..Default::default()
        };
        let p = plan(&spec, &gd);
        assert_eq!(p.bases.len(), 4);
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/newmap_{}",
            std::process::id()
        ));
        let d = build(&p, &spec, &dir, 1, gd.clone(), layers, true);
        let issues = kfx_map::validate::validate(&d.doc, &gd);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == kfx_map::validate::Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            !issues.iter().any(|i| i.code == "keeper-without-heart"),
            "{issues:?}"
        );
        let imps = d
            .doc
            .things
            .items
            .iter()
            .filter(|t| kthings::thing_class(t) == Some(kthings::class::CREATURE))
            .count();
        assert_eq!(imps, 16);
        let gates = d
            .doc
            .things
            .items
            .iter()
            .filter(|t| kthings::is_hero_gate(&gd, t))
            .count();
        assert_eq!(gates, 2);
        assert!(d.is_dirty());
        let s = dk_script(1, &p, &gd);
        assert!(s.contains("COMPUTER_PLAYER(PLAYER3,0)") && !s.contains("COMPUTER_PLAYER(PLAYER0"));
        let l = lua_script(1, &p, &gd);
        assert!(l.contains("ComputerPlayer(PLAYER1, 0)"));
    }
}
