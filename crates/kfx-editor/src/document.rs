//! The edited map: a `kfx_map::mapdoc::MapDoc` plus the game configuration it was loaded
//! with, slab edits with the game's placement rules, and the undo journal.
//!
//! # Edits
//! [`apply_edits`] places slabs the way `MapDoc::place_slab` does
//! (`place_slab_type_on_map_f`, src/map_blocks.c:1646) for a whole batch of cells, regenerates
//! every slab within one of a changed slab once, decorates it ([`Document::decorator`],
//! `kfx_map::decorate`) and replaces the slab objects of the regenerated slabs
//! (`MapDoc::apply_slab_objects`, src/map_blocks.c:590, 1045). Things on the edited slabs follow
//! like `MapDoc::place_slab_with_things` / `set_slab_owner_with_things`.
//!
//! # Undo
//! Every change is a `kfx_map::patch::Patch` (recorded with `MapDoc::record`, or returned by
//! the `kfx_map::things` / `transform` operations). Edits happen inside a stroke
//! ([`Document::begin_stroke`] .. [`Document::end_stroke`]); the patches of one stroke are
//! concatenated into one undo step. Undo reverts the patch, redo applies it: both are exact
//! (the patch stores the old and new cells, columns and items, not a recipe).
use kfx_formats::campaign::{resolve, MapSizeSource, PackKind};
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::levelinfo::Lof;
use kfx_formats::rnc;
use kfx_map::autotile::{self, kinds, SlabRect};
use kfx_map::decorate::Decorator;
use kfx_map::gamedata::{category, slab_flags, ConfigLayer, ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use kfx_map::mods::{CampaignLocations, LayerOrigin, LayerStack, ModsConfig, Stage};
use kfx_map::patch::{Change, Patch};
use kfx_map::things;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `mapNNNNN.<ext>` -> NNNNN.
pub fn map_number(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let d = stem.strip_prefix("map")?;
    (!d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
        .then(|| d.parse().ok())
        .flatten()
}

/// Where a map lives and which configuration it is played with.
#[derive(Clone, Debug)]
pub struct MapLocation {
    pub folder: PathBuf,
    pub level: u32,
    pub dims: MapDims,
    /// Every config layer with its provenance (base, mods, campaign, map; `kfx_map::mods`).
    pub stack: LayerStack,
    pub layers: ConfigLayers,
    /// Campaign / map pack name, `None` for a loose map.
    pub pack: Option<String>,
    /// The campaign / map pack `.cfg` and its kind.
    pub pack_cfg: Option<(PathBuf, PackKind)>,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Size of a loose map: `mapNNNNN.lof` MAPSIZE when it matches the `.slb`, else the `.slb`
/// length (square maps only).
fn loose_dims(folder: &Path, level: u32, slb_len: usize) -> Option<MapDims> {
    let lof = level_bin::find_file(folder, level, "lof")
        .and_then(|p| Lof::load(&p).ok())
        .and_then(|l| l.info().mapsize);
    if let Some((w, h)) = lof {
        if w > 0 && h > 0 && (w * h * 2) as usize == slb_len {
            return Some(MapDims::new(w as u16, h as u16));
        }
    }
    MapDims::from_slb_len(slb_len)
}

/// The campaign / map pack whose levels folder is `folder`: its config layers for `level`,
/// its name and the declared map size of the level (when it lists one).
pub struct PackMatch {
    pub layers: ConfigLayers,
    pub pack: String,
    pub declared: Option<(i32, i32)>,
    pub cfg: PathBuf,
    pub kind: PackKind,
    /// `CONFIGS_LOCATION` / `LEVELS_LOCATION` / `CREATURES_LOCATION` (mods mirror them).
    pub locations: CampaignLocations,
}

/// Campaigns win over map packs; a pack that lists the level wins over one that does not.
pub fn find_pack(install: &Path, folder: &Path, level: u32) -> Option<PackMatch> {
    let mut best: Option<(u8, PackMatch)> = None;
    for kind in [
        PackKind::Campaign,
        PackKind::Mappack,
        PackKind::MultiplayerMappack,
    ] {
        let Some(dir) = kfx_gfx::install::resolve(install, kind.folder()).filter(|d| d.is_dir())
        else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let Ok(r) = resolve(install, &cfg) else {
                continue;
            };
            let Some(ld) = &r.levels_dir else { continue };
            if !same_dir(ld, folder) {
                continue;
            }
            let listed = r.levels.iter().find(|l| l.number == level);
            let rank = match (kind, listed.is_some()) {
                (PackKind::Campaign, true) => 0,
                (_, true) => 1,
                (PackKind::Campaign, false) => 2,
                _ => 3,
            };
            if best.as_ref().is_some_and(|(b, _)| *b <= rank) {
                continue;
            }
            let declared = listed
                .filter(|l| l.mapsize_source != MapSizeSource::Default)
                .map(|l| l.mapsize);
            let layers = ConfigLayers::for_level(
                install,
                r.configs_dir.as_deref(),
                Some(ld),
                r.creatures_dir.as_deref(),
                Some(level),
            );
            let pack = if r.name.is_empty() {
                crate::files::campaign_name(install, &cfg).unwrap_or_else(|| {
                    cfg.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
            } else {
                r.name.clone()
            };
            let locations = CampaignLocations::from_common(&r.common);
            best = Some((
                rank,
                PackMatch {
                    layers,
                    pack,
                    declared,
                    cfg: cfg.clone(),
                    kind,
                    locations,
                },
            ));
        }
    }
    best.map(|(_, m)| m)
}

/// The campaign / map pack `.cfg` whose levels folder is `folder` and that lists `level` in a
/// `[mapNNNNN]` block or its level lists (campaigns first).
pub fn find_pack_cfg(install: &Path, folder: &Path, level: u32) -> Option<PathBuf> {
    for kind in [
        PackKind::Campaign,
        PackKind::Mappack,
        PackKind::MultiplayerMappack,
    ] {
        let Some(dir) = kfx_gfx::install::resolve(install, kind.folder()).filter(|d| d.is_dir())
        else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let Ok(r) = resolve(install, &cfg) else {
                continue;
            };
            if r.levels_dir
                .as_deref()
                .is_some_and(|ld| same_dir(ld, folder))
                && r.levels.iter().any(|l| l.number == level)
            {
                return Some(cfg);
            }
        }
    }
    None
}

/// Base `fxdata` layers, for maps outside any campaign.
pub fn base_layers(install: &Path) -> ConfigLayers {
    ConfigLayers::for_level(install, None, None, None, None)
}

/// The layer stack of a loose map (no campaign): the base, the mods, and the map's own
/// `mapNNNNN.*` overrides in its folder (they apply once the map is played from a map pack).
fn loose_stack(install: &Path, folder: &Path, level: u32, mods: &ModsConfig) -> LayerStack {
    let mut st = LayerStack::build(install, &CampaignLocations::default(), None, mods);
    // the map layer goes before the `after_map` mods
    let pos = |v: &Vec<(LayerOrigin, ConfigLayer)>| {
        v.iter()
            .position(|(o, _)| o.stage == Stage::AfterMap)
            .unwrap_or(v.len())
    };
    let p = pos(&st.config);
    st.config
        .insert(p, (LayerOrigin::map(), ConfigLayer::map(folder, level)));
    let p = pos(&st.creatures);
    st.creatures
        .insert(p, (LayerOrigin::map(), ConfigLayer::map(folder, level)));
    st
}

/// Find a map's campaign / map pack and size, without mods. See [`locate_with`].
pub fn locate(install: &Path, folder: &Path, level: u32) -> Result<MapLocation, String> {
    locate_with(install, folder, level, &ModsConfig::default())
}

/// The configuration of a level (no map file needed): the layer stack with `mods`, the plain
/// layers, the campaign / map pack name and `.cfg`, and the size the pack declares.
pub struct LevelConfig {
    pub stack: LayerStack,
    pub layers: ConfigLayers,
    pub pack: Option<String>,
    pub pack_cfg: Option<(PathBuf, PackKind)>,
    pub declared: Option<(i32, i32)>,
}

pub fn level_config(install: &Path, folder: &Path, level: u32, mods: &ModsConfig) -> LevelConfig {
    match find_pack(install, folder, level) {
        Some(m) => {
            let stack = if mods.is_empty() {
                kfx_map::mods::stack_of_layers(&m.layers)
            } else {
                LayerStack::build(install, &m.locations, Some(level), mods)
            };
            let layers = if mods.is_empty() {
                m.layers.clone()
            } else {
                stack.to_config_layers()
            };
            LevelConfig {
                stack,
                layers,
                pack: Some(m.pack),
                pack_cfg: Some((m.cfg, m.kind)),
                declared: m.declared,
            }
        }
        None => {
            let stack = loose_stack(install, folder, level, mods);
            let layers = stack.to_config_layers();
            LevelConfig {
                stack,
                layers,
                pack: None,
                pack_cfg: None,
                declared: None,
            }
        }
    }
}

/// Find a map's campaign / map pack and size; the config layers include `mods` (what
/// `ModsConfig::for_game` returned; empty = no mods). Loose maps get the base `fxdata` layers
/// (plus their own folder's overrides) and the size of their `.lof` or `.slb`.
pub fn locate_with(
    install: &Path,
    folder: &Path,
    level: u32,
    mods: &ModsConfig,
) -> Result<MapLocation, String> {
    let slb = level_bin::find_file(folder, level, "slb")
        .ok_or_else(|| format!("map{level:05}.slb not found in {}", folder.display()))?;
    let slb_len = rnc::load_file(&slb).map_err(|e| e.to_string())?.len();
    let unknown = || {
        format!(
            "cannot tell the size of map{level:05}: no map size declared and a non-square \
            .slb ({slb_len} bytes)"
        )
    };
    let c = level_config(install, folder, level, mods);
    let declared = c
        .declared
        .filter(|&(w, h)| w > 0 && h > 0 && (w * h * 2) as usize == slb_len)
        .map(|(w, h)| MapDims::new(w as u16, h as u16));
    let dims = declared
        .or_else(|| loose_dims(folder, level, slb_len))
        .ok_or_else(unknown)?;
    Ok(MapLocation {
        folder: folder.to_path_buf(),
        level,
        dims,
        stack: c.stack,
        layers: c.layers,
        pack: c.pack,
        pack_cfg: c.pack_cfg,
    })
}

/// The slab fields an edit changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlabState {
    pub kind: u16,
    pub owner: u8,
    pub wlb: u8,
    pub slx: u8,
}

/// One cell of an edit batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Place a slab kind with an owner (`place_slab` semantics).
    Place { kind: u16, owner: u8 },
    /// Change only the owner (things on the slab follow like in the game).
    Owner(u8),
    /// Set the `.slx` texture style (0 = the level's set, n = set n-1).
    Style(u8),
    /// Place custom slab `id` (sidecar definition) with an owner (`MapDoc::place_custom_slab`).
    Custom { id: u16, owner: u8 },
}

/// What the view has to redraw after a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refresh {
    /// Slabs whose layers or columns changed.
    pub slabs: Option<SlabRect>,
    /// Size, texture set or everything changed: rebuild the whole view.
    pub whole: bool,
    /// Things, action points or lights changed.
    pub items: bool,
}

impl Refresh {
    pub fn merge(self, o: Refresh) -> Refresh {
        Refresh {
            slabs: merge(self.slabs, o.slabs),
            whole: self.whole || o.whole,
            items: self.items || o.items,
        }
    }
}

/// The redraw a patch needs on a map of `dims` (its dims after the patch for whole changes).
pub fn refresh_of(p: &Patch, dims: MapDims) -> Refresh {
    let mut r = Refresh::default();
    for c in &p.changes {
        match c {
            Change::Slabs { rect, .. } => {
                let s = SlabRect {
                    x0: rect.x0 as i32,
                    y0: rect.y0 as i32,
                    x1: rect.x1 as i32,
                    y1: rect.y1 as i32,
                };
                r.slabs = merge(r.slabs, Some(s));
            }
            Change::Subtiles { rect, .. } => {
                let s = SlabRect {
                    x0: rect.x0 as i32 / 3,
                    y0: rect.y0 as i32 / 3,
                    x1: rect.x1 as i32 / 3,
                    y1: rect.y1 as i32 / 3,
                }
                .clip(dims);
                r.slabs = merge(r.slabs, Some(s));
            }
            Change::Thing { .. } | Change::ActionPoint { .. } | Change::Light { .. } => {
                r.items = true
            }
            Change::Props { .. } | Change::Whole { .. } => {
                r.whole = true;
                r.items = true;
            }
        }
    }
    r
}

/// One undo step: a name and the reversible change (`kfx_map::patch`).
#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub patch: Patch,
}

impl Command {
    /// Number of elementary changes.
    pub fn change_count(&self) -> usize {
        self.patch.changes.len()
    }
    /// Approximate memory size in bytes.
    pub fn size(&self) -> usize {
        self.patch.size_hint()
    }
}

struct Stroke {
    name: String,
    patch: Patch,
}

/// Bounding rectangle of a set of slab indices.
fn bounds(dims: MapDims, idx: impl Iterator<Item = u32>) -> Option<SlabRect> {
    let w = dims.w as u32;
    idx.map(|i| SlabRect::single((i % w) as i32, (i / w) as i32))
        .reduce(SlabRect::union)
}

/// A document's decoration settings applied to its decorator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorPrefs {
    pub enabled: bool,
    /// Family overrides of the defaults (name, on).
    pub families: Vec<(String, bool)>,
}

pub struct Document {
    pub doc: MapDoc,
    pub gd: Arc<GameData>,
    pub layers: ConfigLayers,
    /// The layers with their provenance (mods included).
    pub stack: LayerStack,
    pub pack: Option<String>,
    /// The campaign / map pack `.cfg` the level belongs to.
    pub pack_cfg: Option<(PathBuf, PackKind)>,
    /// Record map edits with `MapDoc::record_tracked` (else the snapshot `MapDoc::record`).
    pub tracked: bool,
    /// Cross-check every tracked patch against the snapshot one (`record_checked`; tests).
    pub verify_tracked: bool,
    /// Decoration pass run on every slab the editor (re)generates; seeded per level.
    pub decorator: Decorator,
    /// Random doodad rules (scattered on every slab the editor places or regenerates; saved in
    /// the sidecar by [`Document::save_to_format`]).
    pub doodads: crate::doodads::Doodads,
    /// Rounded corners applied to every slab the editor places or regenerates.
    pub rounding: crate::rounding::Rounding,
    /// `.lof` content of a new map, written with its first save when the folder has no `.lof`.
    pub pending_info: Option<kfx_formats::levelinfo::LevelInfo>,
    /// Backups of files outside the map's own transaction before they are overwritten (the
    /// campaign `.cfg`); the editor shares its session's.
    pub backups: crate::files::SessionBackups,
    journal: Vec<Command>,
    cursor: usize,
    /// Journal position of the last save; `None` when no saved state is reachable.
    saved: Option<usize>,
    stroke: Option<Stroke>,
    /// Bumped on every change of the document (for debounced views such as validation).
    pub revision: u64,
}

impl Document {
    pub fn new(
        doc: MapDoc,
        gd: Arc<GameData>,
        stack: LayerStack,
        pack: Option<String>,
        saved: bool,
    ) -> Document {
        let seed = doc
            .sidecar
            .decoration
            .seed
            .unwrap_or_else(|| Decorator::seed_for_level(doc.level));
        let decorator = Decorator::new(&gd, seed);
        let layers = stack.to_config_layers();
        Document {
            doc,
            gd,
            layers,
            stack,
            pack,
            pack_cfg: None,
            tracked: true,
            verify_tracked: cfg!(test),
            decorator,
            doodads: Default::default(),
            pending_info: None,
            backups: Default::default(),
            rounding: Default::default(),
            journal: vec![],
            cursor: 0,
            saved: saved.then_some(0),
            stroke: None,
            revision: 0,
        }
    }

    /// Load a map located by [`locate`] with its game data.
    pub fn load(loc: &MapLocation, gd: Arc<GameData>) -> Result<Document, String> {
        let doc = MapDoc::load(&loc.folder, loc.level, loc.dims, &gd).map_err(|e| e.to_string())?;
        let mut d = Document::new(doc, gd, loc.stack.clone(), loc.pack.clone(), true);
        d.layers = loc.layers.clone();
        d.pack_cfg = loc.pack_cfg.clone();
        match crate::doodads::Doodads::load(&loc.folder, loc.level) {
            Ok(x) => d.doodads = x,
            Err(e) => d.doc.warnings.push(format!("doodads ignored: {e}")),
        }
        Ok(d)
    }

    /// A new map: `fill` everywhere, rock on the outer ring when `border_rock`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        folder: &Path,
        level: u32,
        dims: MapDims,
        fill: u16,
        border_rock: bool,
        texture: u8,
        gd: Arc<GameData>,
        stack: LayerStack,
    ) -> Document {
        let mut doc = MapDoc::new(folder, level, dims, fill, &gd);
        if border_rock {
            let (w, h) = (dims.w as usize, dims.h as usize);
            for y in 0..h {
                for x in 0..w {
                    if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                        let i = dims.slab_index(x, y);
                        doc.kinds[i] = kinds::ROCK as u16;
                        doc.wlb[i] = gd.slab(0).wlb_type;
                    }
                }
            }
            doc.regenerate_columns(&gd);
        }
        doc.set_texture_id(texture);
        let mut d = Document::new(doc, gd, stack, None, false);
        let dec = d.decorator.clone();
        d.doc.decorate(
            &dec,
            SlabRect {
                x0: 0,
                y0: 0,
                x1: dims.w as i32 - 1,
                y1: dims.h as i32 - 1,
            },
        );
        d
    }

    /// Apply decoration preferences (master switch and families switched off).
    /// The editor-wide preferences, then the map's own decoration settings (sidecar: seed,
    /// master switch, families switched off) on top.
    pub fn set_decor_prefs(&mut self, p: &DecorPrefs) {
        let ds = &self.doc.sidecar.decoration;
        let seed = ds
            .seed
            .unwrap_or_else(|| Decorator::seed_for_level(self.doc.level));
        let mut dec = Decorator::new(&self.gd, seed);
        dec.enabled = p.enabled && ds.enabled;
        for (n, on) in &p.families {
            dec.set_family(n, *on);
        }
        for n in &ds.disabled_families {
            dec.set_family(n, false);
        }
        self.decorator = dec;
    }

    pub fn dims(&self) -> MapDims {
        self.doc.dims
    }

    pub fn in_map(&self, x: i32, y: i32) -> bool {
        autotile::in_map(self.doc.dims, x, y)
    }

    pub fn state_at(&self, x: i32, y: i32) -> Option<SlabState> {
        self.in_map(x, y).then(|| {
            let i = self.doc.dims.slab_index(x as usize, y as usize);
            SlabState {
                kind: self.doc.kinds[i],
                owner: self.doc.owners[i],
                wlb: self.doc.wlb[i],
                slx: self.doc.slx[i],
            }
        })
    }

    // ---------------------------------------------------------------------------- journal

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.cursor) || self.stroke.as_ref().is_some_and(|s| !s.patch.is_empty())
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.cursor);
    }

    pub fn history(&self) -> &[Command] {
        &self.journal
    }

    /// Number of applied steps (the history position).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn undo_name(&self) -> Option<&str> {
        self.cursor
            .checked_sub(1)
            .map(|i| self.journal[i].name.as_str())
    }

    pub fn redo_name(&self) -> Option<&str> {
        self.journal.get(self.cursor).map(|c| c.name.as_str())
    }

    pub fn in_stroke(&self) -> bool {
        self.stroke.is_some()
    }

    /// Start an undo step. An unfinished stroke is ended first.
    pub fn begin_stroke(&mut self, name: &str) {
        self.end_stroke();
        self.stroke = Some(Stroke {
            name: name.to_string(),
            patch: Patch::new(),
        });
    }

    /// Finish the current stroke; returns whether an undo step was recorded.
    pub fn end_stroke(&mut self) -> bool {
        let Some(st) = self.stroke.take() else {
            return false;
        };
        if st.patch.is_empty() {
            return false;
        }
        self.journal.truncate(self.cursor);
        if self.saved.is_some_and(|s| s > self.cursor) {
            self.saved = None;
        }
        self.journal.push(Command {
            name: st.name,
            patch: st.patch,
        });
        self.cursor += 1;
        true
    }

    /// Drop the current stroke, restoring the document; returns what to redraw.
    pub fn cancel_stroke(&mut self) -> Option<Refresh> {
        let st = self.stroke.take()?;
        if st.patch.is_empty() {
            return None;
        }
        st.patch.revert(&mut self.doc);
        self.revision += 1;
        Some(refresh_of(&st.patch, self.doc.dims))
    }

    /// Add an already applied change to the current stroke, or record it as its own undo step
    /// named `name` when no stroke is open. Returns what to redraw.
    pub fn commit(&mut self, name: &str, p: Patch) -> Refresh {
        if p.is_empty() {
            return Refresh::default();
        }
        self.revision += 1;
        let r = refresh_of(&p, self.doc.dims);
        match self.stroke.as_mut() {
            Some(s) => s.patch.extend(p),
            None => {
                self.stroke = Some(Stroke {
                    name: name.to_string(),
                    patch: p,
                });
                self.end_stroke();
            }
        }
        r
    }

    /// Run `f` on the map as one recorded change (see [`Document::commit`]).
    pub fn edit<R>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut MapDoc, &GameData, &Decorator) -> R,
    ) -> (R, Refresh) {
        let gd = self.gd.clone();
        let dec = self.decorator.clone();
        let (r, p) = self.doc.record(|d| f(d, &gd, &dec));
        let rf = self.commit(name, p);
        (r, rf)
    }

    /// [`Document::edit`] recorded with `MapDoc::record_tracked` (no whole-document clone):
    /// `f` must change the map only through `MapDoc` methods or declare its direct writes with
    /// the `touch_*` methods first. Falls back to [`Document::edit`] when `tracked` is off.
    /// Test builds cross-check the patch against the snapshot one (`record_checked`).
    pub fn edit_tracked<R>(
        &mut self,
        name: &str,
        f: impl Fn(&mut MapDoc, &GameData, &Decorator) -> R,
    ) -> (R, Refresh) {
        if !self.tracked {
            return self.edit(name, f);
        }
        let gd = self.gd.clone();
        let dec = self.decorator.clone();
        let (r, p) = if self.verify_tracked {
            self.doc.record_checked(|d| f(d, &gd, &dec))
        } else {
            self.doc.record_tracked(|d| f(d, &gd, &dec))
        };
        let rf = self.commit(name, p);
        (r, rf)
    }

    /// Run one of the `kfx_map::things` operations (they apply and return their patch).
    pub fn op<R>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut MapDoc, &GameData) -> (R, Patch),
    ) -> (R, Refresh) {
        let gd = self.gd.clone();
        let (r, p) = f(&mut self.doc, &gd);
        let rf = self.commit(name, p);
        (r, rf)
    }

    /// Undo one step; returns what to redraw.
    pub fn undo(&mut self) -> Option<Refresh> {
        self.end_stroke();
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.journal[self.cursor].patch.revert(&mut self.doc);
        self.revision += 1;
        Some(refresh_of(&self.journal[self.cursor].patch, self.doc.dims))
    }

    pub fn redo(&mut self) -> Option<Refresh> {
        self.end_stroke();
        if self.cursor >= self.journal.len() {
            return None;
        }
        self.journal[self.cursor].patch.apply(&mut self.doc);
        self.cursor += 1;
        self.revision += 1;
        Some(refresh_of(
            &self.journal[self.cursor - 1].patch,
            self.doc.dims,
        ))
    }

    /// Undo / redo until the history position is `pos`; returns the union of redraws.
    pub fn jump_to(&mut self, pos: usize) -> Option<Refresh> {
        self.end_stroke();
        let pos = pos.min(self.journal.len());
        let mut r: Option<Refresh> = None;
        while self.cursor > pos {
            r = merge_refresh(r, self.undo());
        }
        while self.cursor < pos {
            r = merge_refresh(r, self.redo());
        }
        r
    }

    // ---------------------------------------------------------------------------- edits

    /// Apply a batch of slab edits (inside a stroke; one is started when there is none).
    /// Cells outside the map are ignored. Returns what to redraw, `None` when nothing changed.
    pub fn apply(&mut self, edits: &[(i32, i32, Edit)]) -> Option<Refresh> {
        if self.stroke.is_none() {
            self.begin_stroke("Edit");
        }
        let doodads = &self.doodads;
        let scatter = !doodads.is_empty();
        let doodads = doodads.clone();
        let rounding = self.rounding;
        let (rect, rf) = self.edit_tracked("Edit", |d, gd, dec| {
            let r = apply_edits(d, gd, dec, edits);
            if let Some(r) = r {
                crate::rounding::round_corners(d, &rounding, r, dec.seed);
                if scatter {
                    crate::doodads::scatter(d, gd, &doodads, r);
                }
            }
            r
        });
        rect.map(|_| rf)
    }

    /// Regenerate the slabs of `rect` with the chosen aspects as one undo step.
    pub fn regenerate(&mut self, name: &str, rect: SlabRect, what: Regen) -> Refresh {
        self.end_stroke();
        let doodads = self.doodads.clone();
        let rounding = self.rounding;
        self.edit(name, |d, gd, dec| {
            regenerate(d, gd, dec, rect, what);
            if what.columns {
                crate::rounding::round_corners(d, &rounding, rect, dec.seed);
            }
            if what.doodads {
                crate::doodads::scatter(d, gd, &doodads, rect);
            }
        })
        .1
    }

    /// Scatter the doodads over the whole map with the rules `set` (which become the map's) as
    /// one undo step. Returns the number placed.
    pub fn apply_doodads(&mut self, set: crate::doodads::Doodads) -> (usize, Refresh) {
        self.end_stroke();
        let d = self.doc.dims;
        let all = SlabRect {
            x0: 0,
            y0: 0,
            x1: d.w as i32 - 1,
            y1: d.h as i32 - 1,
        };
        self.doodads = set.clone();
        self.edit("Random doodads", |m, gd, _| {
            crate::doodads::scatter(m, gd, &set, all)
        })
    }

    // ---------------------------------------------------------------------------- files

    /// Write the map to `folder` as level `level` (current columns). Writes a `.lof` with the
    /// map size for non-85x85 maps when the folder has none, so the map can be reopened.
    pub fn save_to(&mut self, folder: &Path, level: u32) -> Result<(), String> {
        self.save_to_format(folder, level, false).map(|_| ())
    }

    /// [`Document::save_to`] in the KeeperFX format, or (`legacy`) the original
    /// `.tng/.apt/.lgt` (`MapDoc::save_legacy`: refused with an error when its report has
    /// errors). Returns the legacy report.
    ///
    /// The map files, the `.lof` and the sidecar (with the doodad rules) are written as one
    /// transaction (all or nothing, `kfx_formats::atomic`). The map size is kept where the game
    /// reads it: a campaign's levels take `MAPSIZE` from the `[mapNNNNN]` block of the campaign
    /// `.cfg` (the `.lof` files of a campaign are not read, src/config_campaigns.c:1326), map packs
    /// and loose maps from `mapNNNNN.lof` (src/lvl_filesdk1.c:589). An existing `.lof` always gets
    /// the true size; one is created for a map that is not 85x85. The campaign `.cfg` is
    /// written after the map (backed up first through [`Document::backups`]).
    pub fn save_to_format(
        &mut self,
        folder: &Path,
        level: u32,
        legacy: bool,
    ) -> Result<Option<kfx_map::legacy::LegacyReport>, String> {
        self.end_stroke();
        let mut doc = self.doc.clone();
        doc.folder = folder.to_path_buf();
        doc.level = level;
        let (mut tx, report) = if legacy {
            let (tx, r) = doc
                .save_legacy_transaction(&self.gd, folder, SaveColumns::Current)
                .map_err(|e| e.to_string())?;
            (tx, Some(r))
        } else {
            (
                doc.save_transaction(&self.gd, folder, SaveColumns::Current)
                    .map_err(|e| e.to_string())?,
                None,
            )
        };
        let d = self.doc.dims;
        let size = (d.w as i32, d.h as i32);
        let default = (
            kfx_formats::levelinfo::DEFAULT_MAP_SIZE,
            kfx_formats::levelinfo::DEFAULT_MAP_SIZE,
        );
        let campaign_cfg = self
            .pack_cfg
            .as_ref()
            .filter(|(_, k)| {
                *k == PackKind::Campaign
                    && same_dir(&self.doc.folder, folder)
                    && self.doc.level == level
            })
            .map(|(p, _)| p.clone());
        match level_bin::find_file(folder, level, "lof") {
            Some(p) => {
                // keep the declared size in step with the map (a resized map, also back to 85x85)
                let mut l = Lof::load(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                let mut info = l.info();
                if info.mapsize.unwrap_or(default) != size {
                    info.mapsize = Some(size);
                    l.set_info(&info);
                    tx.write(level_bin::file_name(level, "lof"), l.to_bytes());
                }
            }
            None => {
                // a new map's level information, or just the size of a map that is not 85x85
                // (a campaign level gets its size in the campaign .cfg instead)
                let mut info = self.pending_info.clone();
                if info.is_none() && size != default && campaign_cfg.is_none() {
                    info = Some(Default::default());
                }
                if let Some(mut info) = info {
                    if size != default {
                        info.mapsize = Some(size);
                    }
                    tx.write(
                        level_bin::file_name(level, "lof"),
                        Lof::new(&info).to_bytes(),
                    );
                }
            }
        }
        self.doodads.add_to(&mut tx, level)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.pending_info = None;
        let mut problems = vec![];
        if let Some(cfg) = &campaign_cfg {
            if let Err(e) = self.update_campaign_mapsize(cfg, level, size) {
                problems.push(e);
            }
        }
        self.doc.folder = folder.to_path_buf();
        self.doc.level = level;
        self.mark_saved();
        if problems.is_empty() {
            Ok(report)
        } else {
            Err(format!("the map was saved, but {}", problems.join("; ")))
        }
    }

    /// Put the map size into the campaign's `[mapNNNNN]` block (what the game reads for
    /// campaign levels): set when the block declares a size or the map is not 85x85.
    fn update_campaign_mapsize(
        &self,
        cfg: &Path,
        level: u32,
        size: (i32, i32),
    ) -> Result<(), String> {
        let default = (
            kfx_formats::levelinfo::DEFAULT_MAP_SIZE,
            kfx_formats::levelinfo::DEFAULT_MAP_SIZE,
        );
        let mut c = kfx_formats::campaign::Campaign::load(cfg)
            .map_err(|e| format!("{}: {e}", cfg.display()))?;
        let mut info = c.map_info(level).unwrap_or_default();
        if info.mapsize.unwrap_or(default) == size {
            return Ok(());
        }
        info.mapsize = Some(size);
        c.set_map_info(level, &info);
        self.backups
            .ensure(cfg)
            .map_err(|e| format!("the campaign's map size was not updated: {e}"))?;
        c.save(cfg).map_err(|e| {
            format!(
                "the campaign's map size was not updated: {}: {e}",
                cfg.display()
            )
        })
    }

    /// Path of the `.slb` this document saves to.
    pub fn slb_path(&self) -> PathBuf {
        level_bin::find_file(&self.doc.folder, self.doc.level, "slb").unwrap_or_else(|| {
            self.doc
                .folder
                .join(level_bin::file_name(self.doc.level, "slb"))
        })
    }

    /// Owned slab count per player 0..=8.
    pub fn owner_counts(&self) -> [usize; 9] {
        let mut c = [0; 9];
        for &o in &self.doc.owners {
            if (o as usize) < 9 {
                c[o as usize] += 1;
            }
        }
        c
    }
}

/// What "regenerate" resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regen {
    /// Columns from the slab sets.
    pub columns: bool,
    /// Decoration pass over the regenerated columns.
    pub decoration: bool,
    /// Torches, room furniture and slab lights.
    pub objects: bool,
    /// `.wlb` from the slab kinds (water / lava; bridges keep theirs) and wibble.
    pub liquid: bool,
    /// Random doodads ([`crate::doodads`]) scattered again.
    pub doodads: bool,
}

impl Default for Regen {
    fn default() -> Self {
        Regen {
            columns: true,
            decoration: true,
            objects: true,
            liquid: true,
            doodads: true,
        }
    }
}

/// Regenerate `rect` (see [`Regen`]).
pub fn regenerate(d: &mut MapDoc, gd: &GameData, dec: &Decorator, rect: SlabRect, what: Regen) {
    let rect = rect.clip(d.dims);
    if rect.is_empty() {
        return;
    }
    if what.liquid {
        for (x, y) in rect.iter() {
            let i = d.dims.slab_index(x as usize, y as usize);
            let s = gd.slab(d.kinds[i] as usize);
            if s.wlb_type != autotile::WLB_BRIDGE && d.custom_cells[i] == 0 {
                d.wlb[i] = s.wlb_type;
            }
        }
    }
    if what.columns {
        let wib: Vec<u8> = d.wibble.clone();
        d.regenerate_rect(gd, rect);
        if !what.liquid {
            // keep the old wibble
            for (x, y) in rect.iter() {
                for k in 0..9 {
                    let s = d
                        .dims
                        .stl_index(3 * x as usize + k % 3, 3 * y as usize + k / 3);
                    d.wibble[s] = wib[s];
                }
            }
        }
    } else if what.liquid {
        let v = d.view(gd);
        let mut wib = vec![];
        for (x, y) in rect.iter() {
            let g = autotile::slab_columns(&v, gd, x, y);
            wib.push((x, y, g.wibble));
        }
        for (x, y, w) in wib {
            for k in 0..9 {
                let s = d
                    .dims
                    .stl_index(3 * x as usize + k % 3, 3 * y as usize + k / 3);
                d.wibble[s] = w[k];
            }
        }
    }
    if what.decoration {
        let mut dec = dec.clone();
        dec.enabled = true;
        d.decorate(&dec, rect);
    }
    if what.objects {
        d.apply_slab_objects(gd, rect);
    }
}

/// `build_door_angle` (src/thing_doors.c:52), index = N<<3 | E<<2 | S<<1 | W.
const BUILD_DOOR_ANGLE: [i8; 16] = [-1, -1, -1, -1, -1, 0, -1, 0, -1, -1, 1, 1, -1, 0, 1, -1];

/// `door_can_stand` (src/thing_doors.c:379): a door on slab (x, y) needs walls (fortified walls,
/// rock, earth, gold, dense gold, gems) on two opposite sides, else the game destroys it on its
/// first update (`process_door`, src/thing_doors.c:574). Outside the map counts as rock.
pub fn door_can_stand(d: &MapDoc, gd: &GameData, x: i32, y: i32) -> bool {
    let mut flags = 0usize;
    for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        flags <<= 1;
        let (sx, sy) = (x + dx, y + dy);
        let wall = if !autotile::in_map(d.dims, sx, sy) {
            true
        } else {
            let k = d.kinds[d.dims.slab_index(sx as usize, sy as usize)] as usize;
            let s = gd.slab(k);
            s.category == category::FORTIFIED_WALL
                || k == kinds::ROCK as usize
                || s.category == category::FRIABLE_DIRT
                || k == kinds::GOLD as usize
                || k == kinds::GEMS as usize
                || s.name.eq_ignore_ascii_case("DENSE_GOLD")
        };
        if wall {
            flags |= 1;
        }
    }
    BUILD_DOOR_ANGLE[flags] != -1
}

/// Checks the editor adds to `kfx_map::validate` (rules the game enforces that it does not
/// check yet).
pub fn extra_issues(d: &MapDoc, gd: &GameData) -> Vec<kfx_map::validate::Issue> {
    use kfx_map::validate::{Issue, Location, Severity};
    let mut v = vec![];
    for (i, t) in d.things.items.iter().enumerate() {
        if things::thing_class(t) != Some(things::class::DOOR) {
            continue;
        }
        let (Some(x), Some(y)) = (t.x, t.y) else {
            continue;
        };
        let (sx, sy) = (x.stl.div_euclid(3), y.stl.div_euclid(3));
        if !door_can_stand(d, gd, sx, sy) {
            v.push(Issue {
                severity: Severity::Error,
                code: "door-cannot-stand",
                message: format!(
                    // door_can_stand, src/thing_doors.c:379
                    "door on slab ({sx},{sy}) has no walls on two opposite sides; \
                    the game destroys it"
                ),
                at: Location::Item(things::ItemRef::Thing(i)),
            });
        }
    }
    v
}

/// Slab kind is a door (`IS_DOOR`).
fn is_door_kind(gd: &GameData, k: u16) -> bool {
    gd.slab(k as usize).block_flags & slab_flags::IS_DOOR != 0
}

/// Re-own the things on slab (x, y) after its owner changed, like
/// `MapDoc::set_slab_owner_with_things` (src/room_data.c:3456, src/room_util.c:353), without its
/// per-slab document snapshot.
fn reown_things(d: &mut MapDoc, gd: &GameData, x: i32, y: i32, owner: u8) {
    let i = d.dims.slab_index(x as usize, y as usize);
    let is_room = gd.slab(d.kinds[i] as usize).block_flags & slab_flags::IS_ROOM != 0;
    let mut remove = vec![];
    let on = d.things_on_slab(x, y);
    if !on.is_empty() {
        d.touch_things();
    }
    for k in on {
        let t = &mut d.things.items[k];
        match things::thing_class(t) {
            Some(things::class::DOOR) => t.owner = Some(owner as i64),
            Some(things::class::OBJECT) if t.subtype == Some(things::OBJ_SPINNING_KEY) => {
                t.owner = Some(owner as i64)
            }
            Some(things::class::OBJECT) if is_room => {
                let m = t.subtype.unwrap_or(-1);
                let base = gd.coloured_object_base(m as i16);
                if base != 0 {
                    t.subtype = Some(
                        gd.coloured_object_model(base, autotile::player_color_idx(owner)) as i64,
                    );
                    t.owner = Some(owner as i64);
                } else if things::object_genre_is(gd, m, "LAIR_TOTEM") {
                    remove.push(k);
                } else if ["SPELLBOOK", "WORKSHOPBOX", "TREASURE_HOARD"]
                    .iter()
                    .any(|g| things::object_genre_is(gd, m, g))
                {
                    t.owner = Some(owner as i64);
                }
            }
            _ => {}
        }
    }
    for k in remove.into_iter().rev() {
        d.things.items.remove(k);
    }
}

/// Things on a slab that changed kind: a non-door kind drops the door and its key, a solid kind
/// buries creatures / objects / traps not attached to it (`place_slab_with_things`).
fn things_after_place(d: &mut MapDoc, gd: &GameData, x: i32, y: i32) {
    let i = d.dims.slab_index(x as usize, y as usize);
    let kind = d.kinds[i];
    if !is_door_kind(gd, kind) {
        if let Some(door) = d.door_on_slab(x, y) {
            d.touch_things();
            d.things.items.remove(door);
            let (cx, cy) = (3 * x + 1, 3 * y + 1);
            if let Some(k) = d.things_at(cx, cy).into_iter().find(|&k| {
                d.things.items[k].subtype == Some(things::OBJ_SPINNING_KEY)
                    && things::thing_class(&d.things.items[k]) == Some(things::class::OBJECT)
            }) {
                d.things.items.remove(k);
            }
        }
    }
    let filled = gd.slab(kind as usize).block_flags & (slab_flags::FILLED | slab_flags::BLOCKING)
        != 0
        && !is_door_kind(gd, kind);
    if filled {
        let slab = autotile::slab_number(d.dims, x, y) as i64;
        let mut buried: Vec<usize> = d
            .things_on_slab(x, y)
            .into_iter()
            .filter(|&k| {
                let t = &d.things.items[k];
                matches!(
                    things::thing_class(t),
                    Some(things::class::CREATURE | things::class::OBJECT | things::class::TRAP)
                ) && t.parent_tile != Some(slab)
            })
            .collect();
        buried.sort();
        if !buried.is_empty() {
            d.touch_things();
        }
        for k in buried.into_iter().rev() {
            d.things.items.remove(k);
        }
    }
}

/// The batch placement: places slabs the way `MapDoc::place_slab` does
/// (`place_slab_type_on_map_f`, src/map_blocks.c:1646): `alter_rock_style` on the placed kind, the
/// slab's `.wlb` from its `WlbType` unless it is a bridge, plain earth around it re-styled
/// (torch dirt). All cells are placed first and every slab within one of a changed slab is
/// regenerated once, with the final layout, then decorated; the slab objects (torches, room
/// furniture, slab lights) of the regenerated slabs are replaced with
/// `MapDoc::apply_slab_objects` like the game does when it re-places those slabs
/// (src/map_blocks.c:590, 1045). Things follow like `place_slab_with_things` /
/// `set_slab_owner_with_things`. Returns the regenerated slabs.
pub fn apply_edits(
    d: &mut MapDoc,
    gd: &GameData,
    dec: &Decorator,
    edits: &[(i32, i32, Edit)],
) -> Option<SlabRect> {
    let dm = d.dims;
    let mut changed: Vec<u32> = vec![];
    let mut placed: Vec<(i32, i32)> = vec![];
    let mut reowned: Vec<(i32, i32, u8)> = vec![];
    let mut restyled: Vec<u32> = vec![];
    for &(x, y, e) in edits {
        if !autotile::in_map(dm, x, y) {
            continue;
        }
        let i = dm.slab_index(x as usize, y as usize);
        match e {
            Edit::Style(v) => {
                if d.slx[i] != v {
                    d.touch_slab(x as usize, y as usize);
                    d.slx[i] = v;
                    restyled.push(i as u32);
                }
            }
            Edit::Owner(o) => {
                if d.owners[i] != o {
                    d.touch_slab(x as usize, y as usize);
                    d.owners[i] = o;
                    changed.push(i as u32);
                    reowned.push((x, y, o));
                }
            }
            Edit::Custom { id, owner } => {
                // place_custom_slab: the kind is the definition's recognized_as, .wlb its liquid
                let Some(def) = d.sidecar.custom_slab(id) else {
                    continue;
                };
                let (k, liquid) = (def.recognized_as, def.liquid);
                let s = gd.slab(k as usize);
                let wlb = match liquid {
                    Some(l) => l,
                    None if s.wlb_type != autotile::WLB_BRIDGE => s.wlb_type,
                    None => d.wlb[i],
                };
                if d.custom_cells[i] == id
                    && d.kinds[i] == k
                    && d.owners[i] == owner
                    && d.wlb[i] == wlb
                {
                    continue;
                }
                d.touch_slab(x as usize, y as usize);
                let owner_changed = d.owners[i] != owner;
                d.kinds[i] = k;
                d.owners[i] = owner;
                d.wlb[i] = wlb;
                d.custom_cells[i] = id;
                changed.push(i as u32);
                placed.push((x, y));
                if owner_changed {
                    reowned.push((x, y, owner));
                }
            }
            Edit::Place { kind, owner } => {
                let k = autotile::alter_rock_style(&d.view(gd), gd, kind as u8, x, y) as u16;
                let k = if kind > u8::MAX as u16 { kind } else { k };
                let s = gd.slab(k as usize);
                let wlb = if s.wlb_type != autotile::WLB_BRIDGE {
                    s.wlb_type
                } else {
                    d.wlb[i]
                };
                if d.kinds[i] == k
                    && d.owners[i] == owner
                    && d.wlb[i] == wlb
                    && d.custom_cells[i] == 0
                {
                    continue;
                }
                d.touch_slab(x as usize, y as usize);
                let owner_changed = d.owners[i] != owner;
                d.kinds[i] = k;
                d.owners[i] = owner;
                d.wlb[i] = wlb;
                d.custom_cells[i] = 0;
                changed.push(i as u32);
                placed.push((x, y));
                if owner_changed {
                    reowned.push((x, y, owner));
                }
                let cat = s.category;
                for (dx, dy) in autotile::AROUND_EIGHT {
                    let (sx, sy) = (x + dx, y + dy);
                    if !autotile::in_map(dm, sx, sy) {
                        continue;
                    }
                    let j = dm.slab_index(sx as usize, sy as usize);
                    let nk = d.kinds[j];
                    if nk != kinds::EARTH as u16 || d.custom_cells[j] != 0 {
                        continue;
                    }
                    let n = if cat == category::ROOM_INTERIOR || cat == category::FORTIFIED_GROUND {
                        if autotile::torch_flags_for_slab(&d.view(gd), sx, sy) == 0 {
                            kinds::EARTH
                        } else {
                            kinds::TORCHDIRT
                        }
                    } else {
                        autotile::alter_rock_style(&d.view(gd), gd, nk as u8, sx, sy)
                    } as u16;
                    if n != nk {
                        d.touch_slab(sx as usize, sy as usize);
                        d.kinds[j] = n;
                        changed.push(j as u32);
                    }
                }
            }
        }
    }
    if changed.is_empty() {
        return bounds(dm, restyled.into_iter());
    }
    // every slab within one of a changed slab is re-placed
    let mut mark = vec![false; dm.slabs()];
    for &i in &changed {
        let (x, y) = ((i % dm.w as u32) as i32, (i / dm.w as u32) as i32);
        for (sx, sy) in SlabRect::single(x, y).grow(1).clip(dm).iter() {
            mark[dm.slab_index(sx as usize, sy as usize)] = true;
        }
    }
    let regen_set: Vec<u32> = (0..dm.slabs() as u32)
        .filter(|&i| mark[i as usize])
        .collect();
    let xy = |i: u32| ((i % dm.w as u32) as i32, (i / dm.w as u32) as i32);
    // custom slabs keep their own nine columns (recoloured for their owner) and are neither
    // regenerated by their neighbours nor decorated
    let cols: Vec<([kfx_formats::level_bin::Column; 9], [u8; 9], bool)> = {
        let v = d.view(gd);
        regen_set
            .iter()
            .map(|&i| {
                let (x, y) = xy(i);
                match d.custom_slab_at(x, y) {
                    Some(c) => {
                        let o = d.owners[i as usize];
                        (
                            std::array::from_fn(|k| autotile::owner_column(gd, &c.columns[k], o)),
                            c.wibble,
                            true,
                        )
                    }
                    None => {
                        let g = autotile::slab_columns(&v, gd, x, y);
                        (g.columns, g.wibble, false)
                    }
                }
            })
            .collect()
    };
    for (&i, c) in regen_set.iter().zip(&cols) {
        let (x, y) = xy(i);
        for k in 0..9 {
            let (sx, sy) = (3 * x as usize + k % 3, 3 * y as usize + k / 3);
            let s = dm.stl_index(sx, sy);
            if d.columns[s] != c.0[k] || d.wibble[s] != c.1[k] {
                d.touch_subtile(sx, sy);
                d.columns[s] = c.0[k];
                d.wibble[s] = c.1[k];
            }
        }
        if !c.2 {
            d.decorate(dec, SlabRect::single(x, y));
        }
    }
    for &(x, y) in &placed {
        things_after_place(d, gd, x, y);
    }
    // slab objects, per horizontal run so that no untouched slab is re-placed (custom slabs keep
    // what they have); runs of consecutive rows with the same extent are merged into one
    // rectangle, because each `apply_slab_objects` call costs a copy of the whole column grid
    // (`MapDoc::slab_objects`), which dominated the stroke latency on large maps
    let mut runs: Vec<SlabRect> = vec![];
    let mut run: Option<SlabRect> = None;
    for (&i, c) in regen_set.iter().zip(&cols) {
        let (x, y) = xy(i);
        if c.2 {
            runs.extend(run.take());
            continue;
        }
        run = match run {
            Some(r) if r.y0 == y && r.x1 + 1 == x => Some(SlabRect { x1: x, ..r }),
            Some(r) => {
                runs.push(r);
                Some(SlabRect::single(x, y))
            }
            None => Some(SlabRect::single(x, y)),
        };
    }
    runs.extend(run);
    for r in merge_rows(runs) {
        d.apply_slab_objects(gd, r);
    }
    for (x, y, o) in reowned {
        reown_things(d, gd, x, y, o);
    }
    merge(
        bounds(dm, regen_set.into_iter()),
        bounds(dm, restyled.into_iter()),
    )
}

/// Merge row runs (in row order) into rectangles: a run joins the rectangle above it when it
/// has the same x extent and lies on the next row. Covers exactly the same slabs.
fn merge_rows(runs: Vec<SlabRect>) -> Vec<SlabRect> {
    let mut out: Vec<SlabRect> = vec![];
    for r in runs {
        if let Some(o) = out
            .iter_mut()
            .rev()
            .find(|o| o.x0 == r.x0 && o.x1 == r.x1 && o.y1 + 1 == r.y0)
        {
            o.y1 = r.y1;
        } else {
            out.push(r);
        }
    }
    out
}

fn merge_refresh(a: Option<Refresh>, b: Option<Refresh>) -> Option<Refresh> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.merge(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn merge(a: Option<SlabRect>, b: Option<SlabRect>) -> Option<SlabRect> {
    match (a, b) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => Some(a.union(b)),
        (Some(a), Some(b)) => Some(if a.is_empty() { b } else { a }),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Files of a level (`mapNNNNN.*`, any case, sorted): the shared listing of `kfx_formats`
/// (Save as, backups, the Level files panel, fingerprints).
pub use kfx_formats::level_bin::{level_file_ext, level_files};

/// Extensions (after `mapNNNNN.`) the map document writes itself (`MapDoc::save`,
/// `save_legacy`, the sidecar) or the script workspace handles.
const DOC_EXTS: &[&str] = &[
    "slb",
    "own",
    "wlb",
    "inf",
    "wib",
    "clm",
    "dat",
    "slx",
    "flg",
    "tng",
    "apt",
    "lgt",
    "tngfx",
    "aptfx",
    "lgtfx",
    "kfxed.toml",
    "une",
    "txt",
    "lua",
];

/// What a level file is for, from the part after `mapNNNNN.` (lower case).
pub fn level_file_role(ext: &str) -> &'static str {
    let e = ext.to_ascii_lowercase();
    match e.as_str() {
        "slb" => "slabs",
        "own" => "ownership",
        "wlb" => "water / lava under bridges",
        "inf" => "texture set",
        "wib" => "wibble",
        "clm" => "column definitions",
        "dat" => "column grid",
        "slx" => "per-slab texture style",
        "flg" => "subtile flags",
        "tng" | "tngfx" => "things",
        "apt" | "aptfx" => "action points",
        "lgt" | "lgtfx" => "lights",
        "lof" => "level overview (name, size, kind)",
        "lif" => "level info (name)",
        "txt" => "DK script",
        "lua" => "Lua script",
        "kfxed.toml" => "editor data (custom slabs, notes, locks)",
        "une" => "custom slab ids (.une format of the Unearth editor)",
        "zip" => "custom sprites",
        "vsn" => "level version",
        _ if e.ends_with(".dat") => "string table",
        _ if e.ends_with(".cfg") || e.ends_with(".toml") => "config override",
        _ => "other",
    }
}

/// Copy the level's own extra files (config overrides `mapNNNNN.rules.cfg`, creature overrides,
/// string tables `mapNNNNN.<lang>.dat`, `mapNNNNN.zip`, `.lof` / `.lif`) to another folder /
/// level number, renamed. Files the document or the scripts write themselves are skipped; a
/// `.lof` / `.lif` already at the target is kept. Returns the files written.
pub fn copy_level_extras(
    from: &Path,
    from_level: u32,
    to: &Path,
    to_level: u32,
) -> Result<Vec<PathBuf>, String> {
    if same_dir(from, to) && from_level == to_level {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for f in level_files(from, from_level) {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(ext) = level_file_ext(&name, from_level) else {
            continue;
        };
        if DOC_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let dest = to.join(format!("map{to_level:05}.{ext}"));
        if matches!(ext.as_str(), "lof" | "lif")
            && level_bin::find_file(to, to_level, &ext).is_some()
        {
            continue;
        }
        if ext == "lif" {
            // a .lif names its level number inside
            let t = kfx_formats::cfg::latin1_decode(
                &std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?,
            );
            let t: String = t
                .lines()
                .map(|l| match l.split_once(',') {
                    Some((n, rest)) if n.trim().parse::<u32>().ok() == Some(from_level) => {
                        format!("{to_level},{rest}\n")
                    }
                    _ => format!("{l}\n"),
                })
                .collect();
            kfx_formats::atomic::atomic_write(&dest, &kfx_formats::cfg::latin1_encode(&t))
                .map_err(|e| format!("{}: {e}", dest.display()))?;
        } else {
            let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            kfx_formats::atomic::atomic_write(&dest, &bytes)
                .map_err(|e| format!("{} -> {}: {e}", f.display(), dest.display()))?;
        }
        out.push(dest);
    }
    Ok(out)
}

/// Copy the files of a level into `<backups>/<timestamp>-mapNNNNN/`; returns that folder,
/// `None` when there was nothing to copy.
pub fn backup_level(folder: &Path, level: u32, backups: &Path) -> Result<Option<PathBuf>, String> {
    let files = level_files(folder, level);
    if files.is_empty() {
        return Ok(None);
    }
    let mut dest = backups.join(format!("{}-map{level:05}", timestamp()));
    let mut n = 1;
    while dest.exists() {
        n += 1;
        dest = backups.join(format!("{}-map{level:05}-{n}", timestamp()));
    }
    std::fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for f in files {
        let to = dest.join(f.file_name().unwrap_or_default());
        std::fs::copy(&f, &to).map_err(|e| format!("{} -> {}: {e}", f.display(), to.display()))?;
    }
    std::fs::write(dest.join("SOURCE.txt"), format!("{}\n", folder.display()))
        .map_err(|e| e.to_string())?;
    Ok(Some(dest))
}

/// Local-time-free UTC timestamp `YYYYMMDD-HHMMSS`.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil_from_days (H. Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
pub mod tests;
