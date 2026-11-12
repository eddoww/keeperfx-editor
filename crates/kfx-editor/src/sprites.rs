//! Thing sprites: which animation the game draws for a thing, at which size, and a lazily
//! filled texture atlas holding the decoded frames.
//!
//! # Sources
//! * Objects: `objects.cfg` `AnimationID` (number or custom sprite name, `value_animid`
//!   src/config_objects.c:89) and `MaximumSize` (`sprite_size_max`, :95, given to `set_thing_draw`
//!   src/thing_objects.c:159).
//! * Creatures: `creatrs/<name>.cfg` `[sprites] Stand` (layered: base folder, campaign
//!   `CREATURES_LOCATION`, `mapNNNNN.<name>.cfg`), size `creature.cfg [common] SpriteSize`
//!   grown by `[experience] SizeIncreaseOnExp` percent per level (src/creature_states.c:1734).
//! * Traps: `trapdoor.cfg` `AnimationID` and `ModelSize` (src/config_trapdoor.c:314-318).
//! * Built-in ids go through `get_td_animation_sprite` ([`crate::fptd`]); custom names through
//!   `kfx_gfx::CustomSet` (fxdata zips, the campaign's config folder, `mapNNNNN.zip`).
//!
//! # Size and anchor
//! In the isometric view the game draws a sprite with `scale = camera_zoom * sprite_size >> 16`
//! (`prepare_jonty_remap_and_scale`, src/engine_render.c:7958-7991, `fade` = 65536 for lens 0) and
//! one sprite pixel covers `scale / 32` screen pixels (`draw_single_keepersprite`,
//! src/engine_render.c:7801-7811), while a subtile (256 map units) covers `zoom * 256 >> 16`
//! pixels (`rotpers_parallel_3`, :541-560). So one sprite pixel is `sprite_size / 8192`
//! subtiles. The bitmap's top-left relative to the thing's position comes from
//! `KeeperSprites::placement` (src/engine_render.c:7653-7670, :7872-7879).
use crate::fptd::td_animation;
use kfx_gfx::custom::CustomSet;
use kfx_gfx::keeper::{view_for_angle, KeeperSprites};
use kfx_gfx::{Palette, Rgba};
use kfx_map::gamedata::{find_ci, parse_number, ConfigLayers, GameData};
use kfx_map::things::class;
use kfx_prefab::i18n::trf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Sprite pixels per subtile at sprite size 1 (see the module doc).
pub const SPRITE_UNITS: f32 = 8192.0;
/// `game.conf.crtr_conf.sprite_size` default (src/config_creature.c:610).
pub const DEFAULT_CREATURE_SIZE: i32 = 300;

/// An animation a thing can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anim {
    /// Built-in: tab index of the first frame, frames, rotable.
    Builtin {
        first: usize,
        frames: u8,
        rotable: u8,
    },
    /// Custom: frame ids per direction (1 or 5 directions).
    Custom { dirs: Vec<Vec<u16>> },
}

impl Anim {
    pub fn frames(&self) -> usize {
        match self {
            Anim::Builtin { frames, .. } => *frames as usize,
            Anim::Custom { dirs } => dirs.first().map_or(0, Vec::len),
        }
    }
    fn rotates(&self) -> bool {
        match self {
            Anim::Builtin { rotable, .. } => *rotable == 2,
            Anim::Custom { dirs } => dirs.len() >= 5,
        }
    }
    /// Frame key (tab index or custom frame id) and mirror for a facing angle relative to the
    /// camera (`resolve_keepersprite_draw_data`, src/engine_render.c:7574-7600).
    pub fn pick(&self, angle: i32, frame: usize) -> Option<(u32, bool)> {
        let n = self.frames();
        if n == 0 {
            return None;
        }
        let f = frame % n;
        let v = view_for_angle(angle);
        let (q, flip) = if self.rotates() {
            (v.quarter as usize, v.flip)
        } else {
            (0, false)
        };
        match self {
            Anim::Builtin { first, frames, .. } => {
                Some(((first + f + q * *frames as usize) as u32, flip))
            }
            Anim::Custom { dirs } => Some((*dirs.get(q).or(dirs.first())?.get(f)? as u32, flip)),
        }
    }
    /// The first frame key (palette icons).
    pub fn first_key(&self) -> Option<u32> {
        self.pick(1024, 0).map(|x| x.0)
    }
}

/// How a thing is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Visual {
    /// A sprite animation at a sprite size; `transparent` is the game's `TransparencyFlags`
    /// (drawn see-through when non-zero).
    Sprite {
        anim: Anim,
        size: i32,
        speed: i32,
        transparent: u8,
    },
    /// No sprite in the game (animation 0 / nothing to draw): a small marker.
    Marker,
    /// The model is not in the loaded configs or its sprite could not be found: "?".
    Unknown,
}

/// A decoded frame ready for the atlas.
pub struct FrameImage {
    pub image: Rgba,
    /// Bitmap top-left relative to the anchor in sprite pixels (y down), unflipped.
    pub x: i32,
    pub y: i32,
}

/// Per-creature data from `<name>.cfg`.
#[derive(Clone, Debug, Default)]
pub struct CreatureInfo {
    pub stand: Option<String>,
    pub query_symbol: Option<String>,
    pub evil: bool,
    /// `[appearance] TransparencyFlags`.
    pub transparent: u8,
    /// Files that contributed.
    pub sources: Vec<PathBuf>,
}

/// Everything needed to draw things of one document.
pub struct SpriteLib {
    pub keeper: Option<KeeperSprites>,
    pub custom: CustomSet,
    pub palette: Palette,
    /// Index = creature model - 1.
    pub creatures: Vec<CreatureInfo>,
    pub creature_size: i32,
    pub size_inc_on_exp: i32,
    visuals: HashMap<(u8, i64, i64), Visual>,
    /// Zips loaded, for the status line.
    pub zips: Vec<String>,
    pub warnings: Vec<String>,
}

fn cfg_int(gd_val: Option<&str>) -> Option<i64> {
    gd_val
        .and_then(|v| v.split_whitespace().next())
        .and_then(parse_number)
}

/// The sprite facts of a creature from its typed config (`GameData::creature_config`: every
/// creature layer, mods included, with provenance).
fn load_creature_info(gd: &GameData, name: &str) -> CreatureInfo {
    let Some(c) = gd.creature_config(name) else {
        return CreatureInfo::default();
    };
    let sref = |r: &kfx_map::creature::SpriteRef| match r {
        kfx_map::creature::SpriteRef::Index(n) => n.to_string(),
        kfx_map::creature::SpriteRef::Name(s) => s.clone(),
    };
    CreatureInfo {
        stand: c.sprite("Stand").map(sref),
        query_symbol: c.query_symbol().map(sref),
        evil: c.is_evil(),
        transparent: c.appearance.transparency_flags.clamp(0, 3) as u8,
        sources: c.cfg.layers.iter().map(|l| l.path.clone()).collect(),
    }
}

impl SpriteLib {
    /// Load the keeper sprites, the custom sprite zips (`fxdata/*.zip` in the game's order,
    /// then the zips of the campaign's config folder, then `mapNNNNN.zip`) and the creature
    /// sprite configs.
    pub fn load(
        install: &kfx_gfx::Install,
        palette: &Palette,
        gd: &GameData,
        layers: &ConfigLayers,
        level_dir: &Path,
        level: u32,
    ) -> SpriteLib {
        let mut warnings = vec![];
        let keeper = match KeeperSprites::load(install) {
            Ok(k) => Some(k),
            Err(e) => {
                warnings.push(trf(
                    "kfx_editor.creature_sprites_e",
                    "creature sprites: {e}",
                    &[("e", &e)],
                ));
                None
            }
        };
        let mut custom = CustomSet::new();
        let mut zips = vec![];
        let fx = install.fxdata_dir();
        for s in custom.load_fxdata(&fx) {
            zips.push(format!("fxdata/{}", s.file));
        }
        // other fxdata zips (mods dropped into fxdata), sorted
        let required: Vec<String> = kfx_gfx::custom::REQUIRED_SPRITE_ZIPS
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        let mut extra: Vec<PathBuf> = std::fs::read_dir(&fx)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
                    .collect()
            })
            .unwrap_or_default();
        extra.retain(|p| {
            !required.contains(
                &p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            )
        });
        extra.sort();
        for p in extra {
            if let Ok(s) = custom.load_zip(&p) {
                zips.push(format!("fxdata/{}", s.file));
            }
        }
        // the campaign's config folder and the mods' folders (layers after the base with no
        // per-map prefix: `fxdata` of a mod, the campaign `CONFIGS_LOCATION`)
        for l in layers.layers.iter().skip(1).filter(|l| l.prefix.is_empty()) {
            for s in custom.load_dir(&l.dir) {
                zips.push(format!("{}/{}", l.dir.display(), s.file));
            }
        }
        if let Some(p) = find_ci(level_dir, &format!("map{level:05}.zip")) {
            match custom.load_zip(&p) {
                Ok(s) => zips.push(s.file),
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            }
        }
        let creatures = gd
            .creatures
            .iter()
            .map(|n| load_creature_info(gd, n))
            .collect();
        let creature_size = cfg_int(gd.creature_cfg.get("common", "SpriteSize"))
            .filter(|&k| k > 0 && k <= 1024)
            .unwrap_or(DEFAULT_CREATURE_SIZE as i64) as i32;
        let size_inc_on_exp =
            cfg_int(gd.creature_cfg.get("experience", "SizeIncreaseOnExp")).unwrap_or(0) as i32;
        SpriteLib {
            keeper,
            custom,
            palette: palette.clone(),
            creatures,
            creature_size,
            size_inc_on_exp,
            visuals: HashMap::new(),
            zips,
            warnings,
        }
    }

    /// `get_anim_id` + `get_td_animation_sprite`: a config value to an animation.
    pub fn anim(&self, value: &str) -> Option<Anim> {
        let v = value.trim().trim_matches('"');
        let r = self.custom.anim_id(v)?;
        if r.id == 0 {
            return None;
        }
        if r.id >= kfx_gfx::custom::KEEPERSPRITE_ADD_OFFSET {
            let a = self
                .custom
                .animation_by_id(r.id)
                .or_else(|| self.custom.animation(v))?;
            return Some(Anim::Custom { dirs: a.td.clone() });
        }
        let id = td_animation(r.id);
        if id == 0 {
            return None;
        }
        let k = self.keeper.as_ref()?;
        let a = k.animation(id)?;
        (a.frames > 0).then_some(Anim::Builtin {
            first: a.first_frame,
            frames: a.frames,
            rotable: a.rotable,
        })
    }

    /// Sprite size of a creature at `level` (1..10).
    pub fn creature_size(&self, level: i64) -> i32 {
        let exp = (level.clamp(1, 10) - 1) as i32;
        self.creature_size + self.creature_size * self.size_inc_on_exp * exp / 100
    }

    /// How a thing of `class` / `model` is drawn (`level` for creatures). Cached.
    pub fn visual(&mut self, gd: &GameData, cls: u8, model: i64, level: i64) -> Visual {
        let key = (cls, model, if cls == class::CREATURE { level } else { 0 });
        if let Some(v) = self.visuals.get(&key) {
            return v.clone();
        }
        let v = self.resolve(gd, cls, model, level);
        self.visuals.insert(key, v.clone());
        v
    }

    fn resolve(&self, gd: &GameData, cls: u8, model: i64, level: i64) -> Visual {
        let sized =
            |anim: Option<Anim>, size: Option<i64>, speed: Option<i64>, tr: Option<i64>| match anim
            {
                Some(anim) => Visual::Sprite {
                    anim,
                    size: size.filter(|&s| s > 0).unwrap_or(300) as i32,
                    speed: speed.unwrap_or(256) as i32,
                    transparent: tr.unwrap_or(0).clamp(0, 3) as u8,
                },
                None => Visual::Unknown,
            };
        match cls {
            class::OBJECT => {
                let sec = format!("object{model}");
                let Some(f) = gd.objects_cfg.section(&sec) else {
                    return Visual::Unknown;
                };
                match f.get("AnimationID").map(str::trim) {
                    None | Some("0") | Some("") => Visual::Marker,
                    Some(a) => sized(
                        self.anim(a),
                        cfg_int(f.get("MaximumSize")),
                        cfg_int(f.get("AnimationSpeed")),
                        cfg_int(f.get("TransparencyFlags")),
                    ),
                }
            }
            class::TRAP => {
                let sec = format!("trap{model}");
                let Some(f) = gd.trapdoor_cfg.section(&sec) else {
                    return Visual::Unknown;
                };
                match f.get("AnimationID").or(f.get("Model")).map(str::trim) {
                    None | Some("0") | Some("") => Visual::Marker,
                    Some(a) => sized(
                        self.anim(a),
                        cfg_int(f.get("ModelSize")),
                        cfg_int(f.get("AnimationSpeed")),
                        cfg_int(f.get("TransparencyFlags")),
                    ),
                }
            }
            class::CREATURE => {
                let Some(info) = (model >= 1)
                    .then(|| self.creatures.get(model as usize - 1))
                    .flatten()
                else {
                    return Visual::Unknown;
                };
                match info.stand.as_deref() {
                    Some(s) => sized(
                        self.anim(s),
                        Some(self.creature_size(level) as i64),
                        Some(256),
                        Some(info.transparent as i64),
                    ),
                    None => Visual::Unknown,
                }
            }
            _ => Visual::Marker,
        }
    }

    /// Decode one frame (tab index or custom frame id) with its placement for `anim`.
    pub fn frame(&self, anim: &Anim, key: u32) -> Option<FrameImage> {
        match anim {
            Anim::Builtin {
                first,
                frames,
                rotable,
            } => {
                let k = self.keeper.as_ref()?;
                let fr = k.decode(key as usize, &self.palette).ok()?;
                let a = kfx_gfx::keeper::Animation {
                    id: 0,
                    first_frame: *first,
                    frames: *frames,
                    rotable: *rotable,
                    directions: 1,
                };
                let (x, y) = k.placement(&a, key as usize, false)?;
                Some(FrameImage {
                    image: fr.image,
                    x,
                    y,
                })
            }
            Anim::Custom { dirs } => {
                let f = self.custom.frame(key as u16)?;
                let first = dirs
                    .first()
                    .and_then(|d| d.first())
                    .and_then(|&id| self.custom.frame(id))
                    .unwrap_or(f);
                Some(FrameImage {
                    image: f.image.clone(),
                    x: first.offset_x + f.offs_w,
                    y: first.offset_y + f.offs_h,
                })
            }
        }
    }

    /// A custom icon by name, or sprite number `n` of `sheet` (gui1-64 for `QuerySymbol`, gui2-64
    /// for `SymbolSprites`); numbers from 950 up are custom icons.
    pub fn icon(&self, value: &str, sheet: Option<&kfx_gfx::SpriteSheet>) -> Option<Rgba> {
        let v = value.split_whitespace().next()?.trim_matches('"');
        if let Some(n) = parse_number(v) {
            if n >= kfx_gfx::sheet::GUI_PANEL_SPRITES_COUNT as i64 {
                return self.custom.icon_image(n as u16).cloned();
            }
            let img = sheet?.decode(n.max(0) as usize, &self.palette).ok()?;
            return (img.w > 0 && img.h > 0).then_some(img);
        }
        let i = self.custom.icon(v)?;
        self.custom.icon_image(i.id).cloned()
    }
}

// ------------------------------------------------------------------------------------------
// Atlas

pub const ATLAS_SIZE: u32 = 4096;

/// Where a frame sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Bitmap top-left relative to the anchor, sprite pixels, unflipped.
    pub ox: i32,
    pub oy: i32,
}

/// Fixed icon ids (stored with keys from [`ICON_BASE`] up).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Unknown,
    Marker,
    ActionPoint,
    Light,
    EffectGen,
    Door,
    Lock,
}

const ICON_BASE: u32 = 0xFFFF_0000;
pub const ICONS: [Icon; 7] = [
    Icon::Unknown,
    Icon::Marker,
    Icon::ActionPoint,
    Icon::Light,
    Icon::EffectGen,
    Icon::Door,
    Icon::Lock,
];

impl Icon {
    fn key(self) -> u32 {
        ICON_BASE + ICONS.iter().position(|&i| i == self).unwrap_or(0) as u32
    }
}

/// Shelf-packed RGBA atlas; new frames are queued for upload.
pub struct SpriteAtlas {
    slots: HashMap<u32, Option<Slot>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// (slot, pixels) waiting for the GPU.
    pub pending: Vec<(Slot, Vec<u8>)>,
    /// The atlas was cleared (everything must be re-uploaded).
    pub generation: u32,
}

impl Default for SpriteAtlas {
    fn default() -> Self {
        SpriteAtlas {
            slots: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            pending: vec![],
            generation: 0,
        }
    }
}

impl SpriteAtlas {
    pub fn clear(&mut self) {
        *self = SpriteAtlas {
            generation: self.generation + 1,
            ..Default::default()
        };
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (pw, ph) = (w + 1, h + 1);
        if pw > ATLAS_SIZE || ph > ATLAS_SIZE {
            return None;
        }
        if self.shelf_x + pw > ATLAS_SIZE {
            self.shelf_y += self.shelf_h;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + ph > ATLAS_SIZE {
            return None;
        }
        let p = (self.shelf_x, self.shelf_y);
        self.shelf_x += pw;
        self.shelf_h = self.shelf_h.max(ph);
        Some(p)
    }

    fn insert(&mut self, key: u32, img: &Rgba, ox: i32, oy: i32) -> Option<Slot> {
        if img.w == 0 || img.h == 0 {
            self.slots.insert(key, None);
            return None;
        }
        let (x, y) = match self.alloc(img.w, img.h) {
            Some(p) => p,
            None => {
                // full: start over (frames in use are added again on demand)
                self.clear();
                self.alloc(img.w, img.h)?
            }
        };
        let s = Slot {
            x,
            y,
            w: img.w,
            h: img.h,
            ox,
            oy,
        };
        self.slots.insert(key, Some(s));
        self.pending.push((s, img.data.clone()));
        Some(s)
    }

    /// The slot of a sprite frame, decoding it on first use.
    pub fn frame(&mut self, lib: &SpriteLib, anim: &Anim, key: u32) -> Option<Slot> {
        if let Some(s) = self.slots.get(&key) {
            return *s;
        }
        match lib.frame(anim, key) {
            Some(f) => self.insert(key, &f.image, f.x, f.y),
            None => {
                self.slots.insert(key, None);
                None
            }
        }
    }

    /// The slot of a built-in icon (drawn on first use).
    pub fn icon(&mut self, icon: Icon) -> Option<Slot> {
        let key = icon.key();
        if let Some(s) = self.slots.get(&key) {
            return *s;
        }
        let img = icon_image(icon);
        let (w, h) = (img.w as i32, img.h as i32);
        self.insert(key, &img, -w / 2, -h)
    }
}

// ------------------------------------------------------------------------------------------
// Procedural icons (no game art for these)

fn put_disc(img: &mut Rgba, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    for y in 0..img.h {
        for x in 0..img.w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy <= r * r {
                img.put(x, y, c);
            }
        }
    }
}

fn put_rect(img: &mut Rgba, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    for y in y0.max(0)..y1.min(img.h as i32) {
        for x in x0.max(0)..x1.min(img.w as i32) {
            img.put(x as u32, y as u32, c);
        }
    }
}

/// 5x7 glyphs for the icons.
const GLYPH_Q: [&str; 7] = [
    ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
];

fn put_glyph(img: &mut Rgba, g: &[&str; 7], x0: i32, y0: i32, s: i32, c: [u8; 4]) {
    for (gy, row) in g.iter().enumerate() {
        for (gx, ch) in row.chars().enumerate() {
            if ch == '#' {
                put_rect(
                    img,
                    x0 + gx as i32 * s,
                    y0 + gy as i32 * s,
                    x0 + (gx as i32 + 1) * s,
                    y0 + (gy as i32 + 1) * s,
                    c,
                );
            }
        }
    }
}

/// The image of a procedural icon (anchor at the bottom centre).
pub fn icon_image(icon: Icon) -> Rgba {
    let black = [10, 8, 6, 255];
    match icon {
        Icon::Unknown => {
            let mut img = Rgba::new(28, 36);
            put_rect(&mut img, 0, 0, 28, 36, black);
            put_rect(&mut img, 2, 2, 26, 34, [250, 40, 200, 255]);
            put_glyph(&mut img, &GLYPH_Q, 4, 4, 4, [255, 255, 255, 255]);
            img
        }
        Icon::Marker => {
            let mut img = Rgba::new(16, 16);
            for y in 0..16 {
                for x in 0..16 {
                    let d = (x as i32 - 8).abs() + (y as i32 - 8).abs();
                    if d <= 7 {
                        img.put(x, y, if d >= 6 { black } else { [200, 200, 210, 255] });
                    }
                }
            }
            img
        }
        Icon::ActionPoint => {
            // a flag on a pole
            let mut img = Rgba::new(24, 40);
            put_rect(&mut img, 3, 0, 7, 40, black);
            put_rect(&mut img, 4, 1, 6, 40, [230, 230, 230, 255]);
            put_rect(&mut img, 6, 1, 24, 15, black);
            put_rect(&mut img, 7, 2, 23, 14, [70, 200, 255, 255]);
            img
        }
        Icon::Light => {
            let mut img = Rgba::new(28, 36);
            put_disc(&mut img, 14.0, 13.0, 13.0, black);
            put_disc(&mut img, 14.0, 13.0, 11.0, [255, 230, 110, 255]);
            put_disc(&mut img, 11.0, 10.0, 4.0, [255, 255, 230, 255]);
            put_rect(&mut img, 8, 24, 20, 36, black);
            put_rect(&mut img, 10, 25, 18, 34, [150, 150, 160, 255]);
            img
        }
        Icon::EffectGen => {
            let mut img = Rgba::new(30, 30);
            for y in 0..30u32 {
                for x in 0..30u32 {
                    let (dx, dy) = (x as f32 - 14.5, y as f32 - 14.5);
                    let a = dy.atan2(dx);
                    let r = (dx * dx + dy * dy).sqrt();
                    let star = 8.0 + 6.0 * (a * 4.0).cos().abs();
                    if r <= star + 1.5 {
                        img.put(x, y, if r > star { black } else { [255, 140, 40, 255] });
                    }
                }
            }
            img
        }
        Icon::Door => {
            let mut img = Rgba::new(24, 32);
            put_rect(&mut img, 0, 0, 24, 32, black);
            put_rect(&mut img, 2, 2, 22, 32, [150, 100, 50, 255]);
            put_rect(&mut img, 11, 4, 13, 32, black);
            put_disc(&mut img, 17.0, 18.0, 2.0, [240, 210, 80, 255]);
            img
        }
        Icon::Lock => {
            let mut img = Rgba::new(20, 24);
            put_disc(&mut img, 10.0, 8.0, 7.5, black);
            put_disc(&mut img, 10.0, 8.0, 4.5, [0, 0, 0, 0]);
            put_rect(&mut img, 0, 10, 20, 24, black);
            put_rect(&mut img, 2, 12, 18, 22, [240, 200, 60, 255]);
            put_rect(&mut img, 9, 14, 11, 20, black);
            img
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn td_conversion() {
        assert_eq!(td_animation(776), 779);
        assert_eq!(td_animation(48), 49);
        assert_eq!(td_animation(930), 931);
        assert_eq!(td_animation(2), 3);
        assert_eq!(td_animation(1), 1);
    }

    #[test]
    fn atlas_packs_and_reuses() {
        let mut a = SpriteAtlas::default();
        let s1 = a.icon(Icon::Unknown).unwrap();
        let s2 = a.icon(Icon::Light).unwrap();
        assert_ne!((s1.x, s1.y), (s2.x, s2.y));
        assert_eq!(a.icon(Icon::Unknown), Some(s1));
        assert_eq!(a.pending.len(), 2);
    }

    /// Shipped sprites resolve: an imp (built-in, rotable), a barrel (object), a druid (custom
    /// zip creature), and the scale rule makes an imp well under a slab and a dragon larger.
    #[test]
    fn shipped_visuals() {
        let Some((root, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let install = kfx_gfx::Install::new(&root);
        let pal = Palette::load(&install).unwrap();
        let mut lib = SpriteLib::load(&install, &pal, &d.gd, &d.layers, &d.doc.folder, 1);
        let gd = d.gd.clone();
        let width = |name: &str, lvl: i64, lib: &mut SpriteLib| -> f32 {
            let m = gd.creature_id(name).unwrap() as i64;
            let Visual::Sprite { anim, size, .. } = lib.visual(&gd, class::CREATURE, m, lvl) else {
                panic!("{name}: no sprite")
            };
            let (key, _) = anim.pick(1024, 0).unwrap();
            let f = lib.frame(&anim, key).unwrap();
            f.image.w as f32 * size as f32 / SPRITE_UNITS / 3.0
        };
        let imp = width("IMP", 1, &mut lib);
        let dragon = width("DRAGON", 1, &mut lib);
        let druid = width("DRUID", 1, &mut lib);
        println!("widths in slabs: imp {imp:.2} dragon {dragon:.2} druid {druid:.2}");
        assert!(imp > 0.1 && imp < 0.7, "imp {imp}");
        assert!(dragon > imp);
        assert!(druid > 0.1);
        assert!(width("IMP", 10, &mut lib) > imp);
        let barrel = gd.object_id("BARREL").unwrap() as i64;
        assert!(matches!(
            lib.visual(&gd, class::OBJECT, barrel, 0),
            Visual::Sprite { .. }
        ));
        assert_eq!(lib.visual(&gd, class::OBJECT, 99999, 0), Visual::Unknown);
    }
}

#[cfg(test)]
mod zip_tests {
    use super::*;

    /// A `mapNNNNN.zip` next to the level is loaded after the fxdata zips.
    #[test]
    fn map_zip_is_loaded() {
        let Some((root, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let install = kfx_gfx::Install::new(&root);
        let pal = Palette::load(&install).unwrap();
        let tmp = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/mapzip_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let Some(src) = kfx_gfx::install::find_in_dir(&install.fxdata_dir(), "druid.zip") else {
            return;
        };
        std::fs::copy(&src, tmp.join("map00001.zip")).unwrap();
        let lib = SpriteLib::load(&install, &pal, &d.gd, &d.layers, &tmp, 1);
        assert!(
            lib.zips
                .iter()
                .any(|z| z.eq_ignore_ascii_case("map00001.zip")),
            "{:?}",
            lib.zips
        );
        assert!(lib.zips.len() >= 10, "{:?}", lib.zips);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}

#[cfg(test)]
mod dump {
    use super::*;
    /// Debug aid: `KFX_DUMP_SPRITES=1` writes some decoded frames to `<temp>/kfx-sprite-*.png`.
    #[test]
    fn dump_frames() {
        if std::env::var("KFX_DUMP_SPRITES").is_err() {
            return;
        }
        let Some((root, d)) = crate::document::tests::load_shipped(10) else {
            return;
        };
        let install = kfx_gfx::Install::new(&root);
        let pal = Palette::load(&install).unwrap();
        let mut lib = SpriteLib::load(&install, &pal, &d.gd, &d.layers, &d.doc.folder, 10);
        for (name, cls, m) in [
            ("imp", class::CREATURE, 23i64),
            ("torch", class::OBJECT, 2),
            ("heartflame", class::OBJECT, 111),
            ("soul", class::OBJECT, 5),
        ] {
            let v = lib.visual(&d.gd, cls, m, 1);
            println!("{name}: {v:?}");
            if let Visual::Sprite { anim, .. } = v {
                for q in [0, 512, 1024] {
                    let (k, _) = anim.pick(q, 0).unwrap();
                    let f = lib.frame(&anim, k).unwrap();
                    println!(
                        "  angle {q} key {k} {}x{} at ({},{})",
                        f.image.w, f.image.h, f.x, f.y
                    );
                    std::fs::write(
                        kfx_formats::testutil::scratch_dir("kfx-editor/sprites")
                            .join(format!("kfx-sprite-{name}-{q}.png")),
                        f.image.to_png().unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }
}
