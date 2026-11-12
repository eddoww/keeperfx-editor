//! Random doodads: user-defined decoration things scattered on chosen slab kinds (Map >
//! Random doodads): each subtile of a matching slab gets a thing with a chance in percent.
//!
//! The rules travel with the map (a `[doodads]` table in the map's
//! sidecar `mapNNNNN.kfxed.toml`, next to what `kfx_map::sidecar` writes there), and the
//! scattering is seeded per slab and subtile, so regenerating a slab gives the same doodads back
//! and an undo / redo / re-apply is stable.
//!
//! Doodads are attached to their slab (`ParentTile` = the slab number), so the game and the
//! editor treat them like the slab's own objects: placing or regenerating the slab removes them
//! (`MapDoc::apply_slab_objects`), and the pass here puts them back.
use kfx_formats::level_fx::{FxFlag, FxLight, StlCoord};
use kfx_map::autotile::{self, SlabRect};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};
use kfx_prefab::i18n::{trd, trf};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// What a rule scatters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DoodadKind {
    /// An object (`objects.cfg` name in [`DoodadRule::model`]).
    #[default]
    Object,
    /// An effect generator (`effects` generator name).
    Effect,
    /// A light (no model; [`DoodadRule::light_range`] and [`DoodadRule::light_intensity`]).
    Light,
}

/// One rule: on every slab of kind `slab`, each of its nine subtiles gets the thing with
/// probability `chance` percent (one roll per subtile).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DoodadRule {
    /// Slab kind name (`terrain.cfg` `Name`), e.g. `WATER`, `PATH`, `LAVA`, `DIRT` (earth).
    pub slab: String,
    pub kind: DoodadKind,
    /// Object or effect generator name (empty for lights).
    pub model: String,
    /// Percent per subtile (0-100).
    pub chance: f32,
    /// Fixed orientation (0-2047), or `None` = one of the eight directions at random.
    pub orientation: Option<i64>,
    /// Lights: range in subtiles and intensity.
    pub light_range: f32,
    pub light_intensity: i64,
    pub enabled: bool,
}

impl Default for DoodadRule {
    fn default() -> Self {
        DoodadRule {
            slab: "PATH".into(),
            kind: DoodadKind::Object,
            model: String::new(),
            chance: 2.0,
            orientation: None,
            light_range: 4.0,
            light_intensity: 32,
            enabled: true,
        }
    }
}

/// The doodad rules of a map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Doodads {
    /// Seed of the scattering (`None`: derived from the level number).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(rename = "rule")]
    pub rules: Vec<DoodadRule>,
}

/// The `[doodads]` table of the sidecar file (other tables belong to `kfx_map::sidecar`).
#[derive(Default, Serialize, Deserialize)]
struct SidecarPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doodads: Option<Doodads>,
}

impl Doodads {
    pub fn is_empty(&self) -> bool {
        self.rules.iter().all(|r| !r.enabled)
    }

    /// Water and lava effect generators at 0.25% (the dialog's "Default effects" button).
    pub fn liquid_effect_defaults(gd: &GameData) -> Doodads {
        let effect = |want: &str| {
            gd.effect_generator_names()
                .into_iter()
                .map(|(_, n)| n)
                .find(|n| n.to_ascii_uppercase().contains(want))
                .unwrap_or_default()
        };
        let rule = |slab: &str, want: &str| DoodadRule {
            slab: slab.into(),
            kind: DoodadKind::Effect,
            model: effect(want),
            chance: 0.25,
            ..Default::default()
        };
        Doodads {
            seed: None,
            rules: vec![rule("WATER", "WATER"), rule("LAVA", "LAVA")],
        }
    }

    /// Read the rules from the sidecar `mapNNNNN.kfxed.toml` in `folder` (none when absent).
    pub fn load(folder: &Path, level: u32) -> Result<Doodads, String> {
        let Some(p) = kfx_formats::level_bin::find_file(folder, level, "kfxed.toml") else {
            return Ok(Doodads::default());
        };
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let part: SidecarPart =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(part.doodads.unwrap_or_default())
    }

    /// The sidecar text with the rules appended as a `[doodads]` table: `existing` is what
    /// `kfx_map` writes (`None` when it writes no sidecar). `None` when there are no rules.
    pub fn sidecar_text(&self, existing: Option<&str>) -> Result<Option<String>, String> {
        if self.rules.is_empty() {
            return Ok(None);
        }
        let mut text = match existing {
            Some(t) => t.to_string(),
            None => format!(
                "# KeeperFX editor sidecar: editor-only data of this map. The game \
                ignores this file.\nversion = {}\n",
                kfx_map::sidecar::SIDECAR_VERSION
            ),
        };
        let part = toml::to_string(&SidecarPart {
            doodads: Some(self.clone()),
        })
        .map_err(|e| e.to_string())?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
        text.push_str(&part);
        Ok(Some(text))
    }

    /// Add the rules to the sidecar of a save transaction (`Document::save_to_format`).
    pub fn add_to(
        &self,
        tx: &mut kfx_formats::atomic::SaveTransaction,
        level: u32,
    ) -> Result<(), String> {
        let name = kfx_formats::level_bin::file_name(level, "kfxed.toml");
        let existing = tx
            .writes()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned());
        if let Some(text) = self.sidecar_text(existing.as_deref())? {
            tx.write(name, text.into_bytes());
        }
        Ok(())
    }

    /// Add the rules to the sidecar of level `level` in `folder`, after `kfx_map` wrote (or
    /// removed) it: appended as a `[doodads]` table, the file created when there was none.
    #[cfg(test)]
    pub fn save(&self, folder: &Path, level: u32) -> Result<(), String> {
        let existing = match kfx_formats::level_bin::find_file(folder, level, "kfxed.toml") {
            Some(p) => {
                Some(std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            }
            None => None,
        };
        let Some(text) = self.sidecar_text(existing.as_deref())? else {
            return Ok(());
        };
        let path = kfx_map::sidecar::sidecar_path(folder, level);
        kfx_formats::atomic::atomic_write(&path, text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn seed(&self, level: u32) -> u64 {
        self.seed.unwrap_or_else(|| {
            kfx_map::decorate::Decorator::seed_for_level(level) ^ 0x444F_4F44_4144
        })
    }
}

/// SplitMix64 finaliser (also the seeded choices of `fortify`).
pub fn mix_pub(z: u64) -> u64 {
    mix(z)
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A rule resolved against the game data.
struct Resolved<'a> {
    rule: &'a DoodadRule,
    index: usize,
    slab: u16,
    class: u8,
    model: i64,
}

fn resolve<'a>(set: &'a Doodads, gd: &GameData) -> Vec<Resolved<'a>> {
    set.rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.enabled && r.chance > 0.0)
        .filter_map(|(index, rule)| {
            let slab = gd.slab_id(&rule.slab)? as u16;
            let (class, model) = match rule.kind {
                DoodadKind::Object => (class::OBJECT, gd.object_id(&rule.model)? as i64),
                DoodadKind::Effect => (
                    class::EFFECT_GEN,
                    gd.effect_generator_names()
                        .into_iter()
                        .find(|(_, n)| n.eq_ignore_ascii_case(&rule.model))?
                        .0 as i64,
                ),
                DoodadKind::Light => (0, 0),
            };
            Some(Resolved {
                rule,
                index,
                slab,
                class,
                model,
            })
        })
        .collect()
}

/// Rules that do not resolve (unknown slab kind or thing name), for the dialog.
pub fn problems(set: &Doodads, gd: &GameData) -> Vec<String> {
    let ok: HashSet<usize> = resolve(set, gd).iter().map(|r| r.index).collect();
    set.rules
        .iter()
        .enumerate()
        .filter(|(i, r)| r.enabled && r.chance > 0.0 && !ok.contains(i))
        .map(|(i, r)| {
            trf(
                "kfx_editor.rule_i_unknown_slab_slab_or_kind_model",
                "rule {i}: unknown slab {slab} or {kind} {model}",
                &[
                    ("i", &(i + 1)),
                    ("slab", &format!("{:?}", r.slab)),
                    (
                        "kind",
                        &(if r.kind == DoodadKind::Effect {
                            trd("kfx_editor.effect", "effect")
                        } else {
                            trd("kfx_editor.object_0ffa", "object")
                        }),
                    ),
                    ("model", &format!("{:?}", r.model)),
                ],
            )
        })
        .collect()
}

/// (Re)scatter the doodads on the slabs of `rect`: the doodads of the rules of each slab's
/// kind already there are removed, then every subtile rolls (seeded per map, slab, subtile and
/// rule). Custom slabs get none; a subtile that already holds a thing is skipped. Declares its
/// writes with `touch_*` (safe inside `MapDoc::record_tracked`). Returns the number placed.
pub fn scatter(d: &mut MapDoc, gd: &GameData, set: &Doodads, rect: SlabRect) -> usize {
    let rect = rect.clip(d.dims);
    if rect.is_empty() {
        return 0;
    }
    let rules = resolve(set, gd);
    if rules.is_empty() {
        return 0;
    }
    let dm = d.dims;
    let on = |d: &MapDoc, x: i32, y: i32| -> Vec<&Resolved> {
        let i = dm.slab_index(x as usize, y as usize);
        if d.custom_cells[i] != 0 {
            return vec![];
        }
        rules.iter().filter(|r| r.slab == d.kinds[i]).collect()
    };
    // remove the old doodads of these rules (things and lights attached to the slab)
    let parent = |x: i32, y: i32| autotile::slab_number(dm, x, y) as i64;
    let inside = |sx: i32, sy: i32, x: i32, y: i32| sx.div_euclid(3) == x && sy.div_euclid(3) == y;
    let mut kill_things: HashSet<usize> = HashSet::new();
    let mut kill_lights: HashSet<usize> = HashSet::new();
    let mut any = false;
    for (x, y) in rect.iter() {
        let rs = on(d, x, y);
        if rs.is_empty() {
            continue;
        }
        any = true;
        let p = parent(x, y);
        for (k, t) in d.things.items.iter().enumerate() {
            let (Some(c), Some((sx, sy))) = (things::thing_class(t), things::position(t.x, t.y))
            else {
                continue;
            };
            if t.parent_tile == Some(p)
                && inside(sx, sy, x, y)
                && rs
                    .iter()
                    .any(|r| r.class == c && Some(r.model) == t.subtype)
            {
                kill_things.insert(k);
            }
        }
        if rs.iter().any(|r| r.rule.kind == DoodadKind::Light) {
            for (k, l) in d.lights.items.iter().enumerate() {
                if l.parent_tile == Some(p)
                    && things::position(l.x, l.y).is_some_and(|(sx, sy)| inside(sx, sy, x, y))
                {
                    kill_lights.insert(k);
                }
            }
        }
    }
    if !any {
        return 0;
    }
    d.touch_things();
    d.touch_lights();
    if !kill_things.is_empty() {
        let mut k = 0;
        d.things.items.retain(|_| {
            k += 1;
            !kill_things.contains(&(k - 1))
        });
    }
    if !kill_lights.is_empty() {
        let mut k = 0;
        d.lights.items.retain(|_| {
            k += 1;
            !kill_lights.contains(&(k - 1))
        });
    }
    // subtiles that hold something already
    let mut used: HashSet<(i32, i32)> = d
        .things
        .items
        .iter()
        .filter_map(|t| things::position(t.x, t.y))
        .collect();
    used.extend(
        d.lights
            .items
            .iter()
            .filter_map(|l| things::position(l.x, l.y)),
    );
    let seed = set.seed(d.level);
    let mut placed = 0;
    for (x, y) in rect.iter() {
        let rs: Vec<(u8, i64, DoodadKind, f32, Option<i64>, f32, i64, usize)> = on(d, x, y)
            .iter()
            .map(|r| {
                (
                    r.class,
                    r.model,
                    r.rule.kind,
                    r.rule.chance,
                    r.rule.orientation,
                    r.rule.light_range,
                    r.rule.light_intensity,
                    r.index,
                )
            })
            .collect();
        if rs.is_empty() {
            continue;
        }
        let owner = d.owners[dm.slab_index(x as usize, y as usize)];
        let p = parent(x, y);
        for (class, model, kind, chance, orientation, range, intensity, index) in rs {
            for k in 0..9 {
                let (sx, sy) = (3 * x + k % 3, 3 * y + k / 3);
                let h = mix(seed
                    ^ mix(((x as u64) << 40)
                        ^ ((y as u64) << 20)
                        ^ ((k as u64) << 12)
                        ^ index as u64));
                if (h % 1_000_000) as f32 / 10_000.0 >= chance {
                    continue;
                }
                if used.contains(&(sx, sy)) {
                    continue;
                }
                used.insert((sx, sy));
                let floor = d.floor_height(sx, sy) as i32;
                if kind == DoodadKind::Light {
                    d.lights.items.push(FxLight {
                        dynamic: Some(FxFlag::Bool(false)),
                        x: Some(things::centred(sx)),
                        y: Some(things::centred(sy)),
                        z: Some(StlCoord::new(floor + 2, 0)),
                        range: Some(StlCoord::new(
                            range.trunc() as i32,
                            (range.fract() * 256.0) as i32,
                        )),
                        intensity: Some(intensity),
                        parent_tile: Some(p),
                        ..Default::default()
                    });
                } else {
                    let mut t = things::new_thing(
                        class,
                        model,
                        owner,
                        things::centred(sx),
                        things::centred(sy),
                        StlCoord::new(floor, 0),
                    );
                    t.parent_tile = Some(p);
                    t.orientation = Some(orientation.unwrap_or(((h >> 32) % 8) as i64 * 256));
                    d.things.items.push(t);
                }
                placed += 1;
            }
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(slab: &str, model: &str, chance: f32) -> DoodadRule {
        DoodadRule {
            slab: slab.into(),
            kind: DoodadKind::Object,
            model: model.into(),
            chance,
            ..Default::default()
        }
    }

    #[test]
    fn scatter_is_seeded_idempotent_and_saved() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let gd = d.gd.clone();
        let mut doc = d.doc.clone();
        let obj = gd
            .object_names()
            .into_iter()
            .map(|(_, n)| n)
            .find(|n| n.eq_ignore_ascii_case("BARREL"))
            .or_else(|| gd.object_names().into_iter().nth(2).map(|x| x.1))
            .unwrap();
        let set = Doodads {
            seed: Some(7),
            rules: vec![rule("DIRT", &obj, 10.0)],
        };
        assert!(problems(&set, &gd).is_empty(), "{:?}", problems(&set, &gd));
        let all = SlabRect {
            x0: 0,
            y0: 0,
            x1: doc.dims.w as i32 - 1,
            y1: doc.dims.h as i32 - 1,
        };
        let before = doc.things.items.len();
        let n = scatter(&mut doc, &gd, &set, all);
        assert!(n > 100, "placed {n}");
        assert_eq!(doc.things.items.len(), before + n);
        // stable: scattering again replaces the same doodads
        let snapshot = doc.things.items.clone();
        let n2 = scatter(&mut doc, &gd, &set, all);
        assert_eq!(n, n2);
        assert_eq!(doc.things.items.len(), snapshot.len());
        // every doodad is attached to an earth slab and stands on its own subtile
        let id = gd.object_id(&obj).unwrap() as i64;
        let dd: Vec<_> = doc
            .things
            .items
            .iter()
            .filter(|t| t.subtype == Some(id) && t.parent_tile.is_some())
            .collect();
        let mut seen = HashSet::new();
        for t in &dd {
            let (sx, sy) = things::position(t.x, t.y).unwrap();
            assert!(seen.insert((sx, sy)));
            assert_eq!(
                doc.kinds[doc.dims.slab_index((sx / 3) as usize, (sy / 3) as usize)],
                kfx_map::autotile::kinds::EARTH as u16
            );
        }
        // another seed gives another pattern
        let mut doc2 = d.doc.clone();
        scatter(
            &mut doc2,
            &gd,
            &Doodads {
                seed: Some(8),
                ..set.clone()
            },
            all,
        );
        assert_ne!(doc2.things.items, doc.things.items);
        // sidecar round trip next to kfx_map's own tables
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/kfx-doodads-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = d.doc.clone();
        m.sidecar.notes.push(kfx_map::sidecar::Note {
            x: 1,
            y: 2,
            text: "hi".into(),
            color: None,
        });
        m.save_sidecar(&dir).unwrap();
        set.save(&dir, 1).unwrap();
        assert_eq!(Doodads::load(&dir, 1).unwrap(), set);
        let (sc, _, w) =
            kfx_map::sidecar::load(&kfx_map::sidecar::sidecar_path(&dir, 1), m.dims).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(sc.notes.len(), 1);
        // without kfx_map's file
        let dir2 = dir.join("b");
        std::fs::create_dir_all(&dir2).unwrap();
        set.save(&dir2, 3).unwrap();
        assert_eq!(Doodads::load(&dir2, 3).unwrap(), set);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
