//! Custom sprite zips: `fxdata/*.zip`, campaign zips and `mapNNNNN.zip`.
//!
//! A zip holds `sprites.json` (animations), `icons.json` (GUI icons) and PNG files
//! (`custom_sprites.c`: `load_file_sprites` :232-309, `process_sprite_from_list` :1470-1530,
//! `collect_sprites`/`read_png_data` :930-1100 and :1340-1458, `process_icon_from_list`
//! :2038-2105). Pixels stay true colour here; the game quantises them to the palette at load time
//! (`compress_raw`, :1170-1250) which we do not do.
//!
//! Id assignment is deterministic from load order and reproduced exactly:
//! * every decoded animation frame takes the next free slot (`next_free_sprite`, :977-985); ids are
//!   `KEEPERSPRITE_ADD_OFFSET + slot`. Per animation all top-down frames are loaded first (all
//!   directions in order), then the first-person ones; the animation's id is its first top-down
//!   frame. A frame that fails to decode after earlier ones succeeded keeps its slots (the game
//!   never returns them).
//! * every icon PNG takes the next icon slot; its id is `GUI_PANEL_SPRITES_COUNT + slot`
//!   (:748-754).
//!
//! Callers must feed zips in the game's order for ids to match: the required `fxdata` zips in the
//! order of [`REQUIRED_SPRITE_ZIPS`] (`src/custom_sprites.h:33-43`), then mods / campaign dir zips /
//! the map zip. Directory enumeration order of the game (`LbFileFindFirst`) is filesystem
//! dependent, so ids for campaign-directory zips cannot be reproduced reliably; look up by name.
use crate::image::Rgba;
use crate::sheet::GUI_PANEL_SPRITES_COUNT;
use crate::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub use crate::keeper::KEEPERSPRITE_ADD_OFFSET;

/// Maximum custom animation frame slots (`KEEPERSPRITE_ADD_NUM = 65535 - 16384`).
pub const KEEPERSPRITE_ADD_NUM: usize = 65535 - KEEPERSPRITE_ADD_OFFSET as usize;
/// Maximum custom icons (`GUI_PANEL_SPRITES_NEW`, src/sprites.h:780).
pub const GUI_PANEL_SPRITES_NEW: usize = 512;

/// Zips the game always loads from `fxdata`, in this order (`src/custom_sprites.h:33-43`).
pub const REQUIRED_SPRITE_ZIPS: [&str; 9] = [
    "colored_sprites.zip",
    "creatures.zip",
    "decorative_objects.zip",
    "druid.zip",
    "effects.zip",
    "maiden.zip",
    "natural_features.zip",
    "time_mage.zip",
    "trapsdoors.zip",
];

/// One animation frame with the placement fields of `struct KeeperSprite` (see `read_png_data`).
#[derive(Clone, Debug)]
pub struct CustomFrame {
    pub image: Rgba,
    pub frame_w: u32,
    pub frame_h: u32,
    /// `FrameOffsW/H`: bitmap position inside the frame box.
    pub offs_w: i32,
    pub offs_h: i32,
    /// Frame box origin relative to the anchor (JSON `offset_x` negated, as the game stores it).
    pub offset_x: i32,
    pub offset_y: i32,
    pub shadow_offset: i32,
    pub frame_flags: u32,
}

/// A named custom animation.
#[derive(Clone, Debug)]
pub struct CustomAnim {
    pub name: String,
    /// Id of the first top-down frame (what `get_anim_id` returns).
    pub id: u16,
    /// Id of the first first-person frame.
    pub fp_id: u16,
    pub rotable: bool,
    /// Top-down frame ids per direction (1 list, or 5 when rotable).
    pub td: Vec<Vec<u16>>,
    pub fp: Vec<Vec<u16>>,
    /// Zip file name it came from.
    pub source: String,
}

impl CustomAnim {
    /// Frames per direction (of direction 0).
    pub fn frames(&self) -> usize {
        self.td.first().map_or(0, |d| d.len())
    }
}

/// A named custom icon (possibly several frames, e.g. big + small).
#[derive(Clone, Debug)]
pub struct CustomIcon {
    pub name: String,
    /// Id of the first frame (`>= GUI_PANEL_SPRITES_COUNT`).
    pub id: u16,
    pub frames: usize,
    pub source: String,
}

/// Result of `get_anim_id`-style name resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimRef {
    pub id: u16,
    /// From the `NAME:DIRECTION` form: 0 NORTH, 1 NORTHEAST ... 7 NORTHWEST.
    pub direction: Option<u8>,
}

/// What loading one zip added.
#[derive(Clone, Debug, Default)]
pub struct ZipSummary {
    pub file: String,
    pub animations: usize,
    pub icons: usize,
    pub warnings: Vec<String>,
}

/// All custom sprites of a load session, in the game's id space.
#[derive(Default)]
pub struct CustomSet {
    slots: Vec<CustomFrame>,
    icon_frames: Vec<Rgba>,
    anims: Vec<CustomAnim>,
    icons: Vec<CustomIcon>,
    anim_by_name: HashMap<String, usize>,
    icon_by_name: HashMap<String, usize>,
    pub warnings: Vec<String>,
}

fn atoi(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    let v: i64 = digits.parse().unwrap_or(0);
    if neg {
        -v
    } else {
        v
    }
}

fn direction_from_name(s: &str) -> Option<u8> {
    [
        "NORTH",
        "NORTHEAST",
        "EAST",
        "SOUTHEAST",
        "SOUTH",
        "SOUTHWEST",
        "WEST",
        "NORTHWEST",
    ]
    .iter()
    .position(|&n| n == s)
    .map(|p| p as u8)
}

fn jint(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_bool().map(|b| b as i64))
}

fn read_entry(
    zip: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
    name: &str,
) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    if let Ok(mut f) = zip.by_name(name) {
        f.read_to_end(&mut buf).ok()?;
        return Some(buf);
    }
    let found = zip
        .file_names()
        .find(|n| n.eq_ignore_ascii_case(name))
        .map(str::to_owned)?;
    let mut f = zip.by_name(&found).ok()?;
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn decode_png(
    zip: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
    name: &str,
) -> std::result::Result<Rgba, String> {
    let bytes = read_entry(zip, name).ok_or_else(|| format!("png '{name}' not found"))?;
    let img = Rgba::from_png(&bytes).map_err(|e| format!("'{name}': {e}"))?;
    if img.w >= 255 || img.h >= 255 {
        return Err(format!(
            "'{name}': sprites of 255x255 or more are not supported ({}x{})",
            img.w, img.h
        ));
    }
    Ok(img)
}

impl CustomSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the required `fxdata` zips (those that exist) in the game's order.
    pub fn load_fxdata(&mut self, fxdata_dir: &Path) -> Vec<ZipSummary> {
        let mut out = Vec::new();
        for name in REQUIRED_SPRITE_ZIPS {
            if let Some(p) = crate::install::find_in_dir(fxdata_dir, name) {
                match self.load_zip(&p) {
                    Ok(s) => out.push(s),
                    Err(e) => self.warnings.push(format!("{name}: {e}")),
                }
            } else {
                self.warnings
                    .push(format!("required fxdata/{name} is missing"));
            }
        }
        out
    }

    /// Load every `*.zip` of a directory, sorted by name (the game's order is filesystem
    /// dependent). Used for campaign / mod directories.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<ZipSummary> {
        let mut zips: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
                    .collect()
            })
            .unwrap_or_default();
        zips.sort();
        zips.iter().filter_map(|p| self.load_zip(p).ok()).collect()
    }

    /// Load one zip: `sprites.json` then `icons.json` (the game's order in `load_file_sprites`).
    /// Problems with individual entries become warnings; an unreadable zip is an error.
    pub fn load_zip(&mut self, path: &Path) -> Result<ZipSummary> {
        let file = std::fs::File::open(path).map_err(|e| kfx_formats::Error::io(path, e))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| Error::format("zip", format!("{}: {e}", path.display())))?;
        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut sum = ZipSummary {
            file: fname.clone(),
            ..Default::default()
        };
        let warn = |sum: &mut ZipSummary, m: String| sum.warnings.push(format!("{fname}: {m}"));

        for (json_name, is_icons) in [("sprites.json", false), ("icons.json", true)] {
            let Some(bytes) = read_entry(&mut zip, json_name) else {
                continue;
            };
            if bytes.len() >= 1024 * 1024 {
                warn(&mut sum, format!("{json_name} too big"));
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let text = text.trim_start_matches('\u{feff}');
            let root: Value = match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    warn(&mut sum, format!("{json_name}: {e}"));
                    continue;
                }
            };
            let Some(arr) = root.as_array() else {
                warn(
                    &mut sum,
                    format!("{json_name} should be an array of dictionaries"),
                );
                continue;
            };
            for (i, item) in arr.iter().enumerate() {
                let r = if is_icons {
                    self.add_icon(&mut zip, &fname, item)
                } else {
                    self.add_sprite(&mut zip, &fname, item)
                };
                match r {
                    Ok(()) => {
                        if is_icons {
                            sum.icons += 1
                        } else {
                            sum.animations += 1
                        }
                    }
                    Err(m) => warn(&mut sum, format!("{json_name}[{i}]: {m}")),
                }
            }
        }
        self.warnings.extend(sum.warnings.iter().cloned());
        Ok(sum)
    }

    fn add_sprite(
        &mut self,
        zip: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
        source: &str,
        node: &Value,
    ) -> std::result::Result<(), String> {
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .ok_or("no \"name\" key")?
            .to_owned();
        let rotatable = node.get("rotatable").and_then(jint).is_some_and(|v| v != 0);
        let dirs = if rotatable { 5 } else { 1 };
        let mut lists: [Vec<Vec<u16>>; 2] = [Vec::new(), Vec::new()];
        // top-down first, first-person second: the order slots are taken in (:1344-1347)
        for (pass, is_fp) in [false, true].into_iter().enumerate() {
            let key = if is_fp { "fp" } else { "td" };
            let ud = node.get(key).and_then(Value::as_array);
            for lr in 0..dirs {
                let mut ids = Vec::new();
                let items = ud.and_then(|u| u.get(lr)).and_then(Value::as_array);
                for itm in items.map(|v| v.as_slice()).unwrap_or(&[]) {
                    let file = itm
                        .get("file")
                        .and_then(Value::as_str)
                        .ok_or("invalid sprite file record")?;
                    let img = decode_png(zip, file)?;
                    if self.slots.len() >= KEEPERSPRITE_ADD_NUM {
                        return Err("too many custom sprites allocated".into());
                    }
                    let fr = Self::make_frame(img, itm, node, is_fp);
                    ids.push(KEEPERSPRITE_ADD_OFFSET + self.slots.len() as u16);
                    self.slots.push(fr);
                }
                lists[pass].push(ids);
            }
        }
        let [td, fp] = lists;
        let td_n: usize = td.iter().map(Vec::len).sum();
        let fp_n: usize = fp.iter().map(Vec::len).sum();
        if td_n == 0 {
            return Err("no top-down frames".into());
        }
        if td_n != fp_n {
            return Err(format!(
                "should have same amount of TD and FP frames ({td_n} vs {fp_n})"
            ));
        }
        let first = |l: &Vec<Vec<u16>>| l.iter().flatten().next().copied().unwrap_or(0);
        let anim = CustomAnim {
            name: name.clone(),
            id: first(&td),
            fp_id: first(&fp),
            rotable: rotatable,
            td,
            fp,
            source: source.to_owned(),
        };
        let key = name.to_ascii_lowercase();
        if self.anim_by_name.contains_key(&key) {
            self.warnings.push(format!(
                "{source}: sprite '{name}' overwrites sprite with same name"
            ));
        }
        self.anim_by_name.insert(key, self.anims.len());
        self.anims.push(anim);
        Ok(())
    }

    /// `READ_WITH_DEFAULT` of read_png_data (:1050-1085): item key, else `fp_`/`td_` key of the
    /// animation, else the default.
    fn make_frame(img: Rgba, itm: &Value, def: &Value, is_fp: bool) -> CustomFrame {
        let pre = if is_fp { "fp_" } else { "td_" };
        let get = |k: &str| -> Option<i64> {
            itm.get(k)
                .filter(|v| !v.is_null())
                .and_then(jint)
                .or_else(|| {
                    def.get(format!("{pre}{k}"))
                        .filter(|v| !v.is_null())
                        .and_then(jint)
                })
        };
        let (w, h) = (img.w as i32, img.h as i32);
        let offset_x = get("offset_x").map(|v| -(v as i32)).unwrap_or(-w / 2);
        let offset_y = get("offset_y").map(|v| -(v as i32)).unwrap_or(1 - h);
        CustomFrame {
            frame_w: get("frame_w").map_or(img.w, |v| v as u32),
            frame_h: get("frame_h").map_or(img.h, |v| v as u32),
            offs_w: get("offset_w").unwrap_or(0) as i32,
            offs_h: get("offset_h").unwrap_or(0) as i32,
            offset_x,
            offset_y,
            shadow_offset: get("shadow_offset").map_or(h + offset_y, |v| v as i32),
            frame_flags: get("frame_flags").unwrap_or(0) as u32,
            image: img,
        }
    }

    fn add_icon(
        &mut self,
        zip: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
        source: &str,
        node: &Value,
    ) -> std::result::Result<(), String> {
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .ok_or("no \"name\" key")?
            .to_owned();
        // The game picks "lowres" on small screens; we always use the full size "file".
        let files: Vec<String> = match node.get("file") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => return Err("invalid or missing \"file\"".into()),
        };
        let mut first = 0u16;
        for f in &files {
            let img = decode_png(zip, f)?;
            if self.icon_frames.len() >= GUI_PANEL_SPRITES_NEW {
                return Err("too many custom icons allocated".into());
            }
            let id = GUI_PANEL_SPRITES_COUNT + self.icon_frames.len() as u16;
            self.icon_frames.push(img);
            if first == 0 {
                first = id;
            }
        }
        let key = name.to_ascii_lowercase();
        if self.icon_by_name.contains_key(&key) {
            self.warnings.push(format!(
                "{source}: icon '{name}' overrides icon with same name"
            ));
        }
        self.icon_by_name.insert(key, self.icons.len());
        self.icons.push(CustomIcon {
            name,
            id: first,
            frames: files.len(),
            source: source.to_owned(),
        });
        Ok(())
    }

    /// All animations in load order (including ones later shadowed by a same-named one).
    pub fn animations(&self) -> &[CustomAnim] {
        &self.anims
    }

    pub fn icons(&self) -> &[CustomIcon] {
        &self.icons
    }

    /// Total frame slots used (`next_free_sprite`).
    pub fn slots_used(&self) -> usize {
        self.slots.len()
    }

    /// Animation by exact name (case-insensitive); the latest loaded wins.
    pub fn animation(&self, name: &str) -> Option<&CustomAnim> {
        self.anim_by_name
            .get(&name.to_ascii_lowercase())
            .map(|&i| &self.anims[i])
    }

    /// Animation whose first top-down frame has id `id`.
    pub fn animation_by_id(&self, id: u16) -> Option<&CustomAnim> {
        self.anim_by_name
            .values()
            .map(|&i| &self.anims[i])
            .find(|a| a.id == id)
    }

    /// Frame image/placement for a frame id >= 16384.
    pub fn frame(&self, id: u16) -> Option<&CustomFrame> {
        self.slots
            .get(id.checked_sub(KEEPERSPRITE_ADD_OFFSET)? as usize)
    }

    /// Icon by exact name (case-insensitive), latest wins.
    pub fn icon(&self, name: &str) -> Option<&CustomIcon> {
        self.icon_by_name
            .get(&name.to_ascii_lowercase())
            .map(|&i| &self.icons[i])
    }

    /// Image of a custom icon frame id (`>= GUI_PANEL_SPRITES_COUNT`).
    pub fn icon_image(&self, id: u16) -> Option<&Rgba> {
        self.icon_frames
            .get(id.checked_sub(GUI_PANEL_SPRITES_COUNT)? as usize)
    }

    /// `get_anim_id` (src/custom_sprites.c:2803-2876): a positive number is the id itself; else a
    /// custom animation name; else `NAME:DIRECTION` (NORTH, NORTHEAST, EAST, SOUTHEAST, SOUTH,
    /// SOUTHWEST, WEST, NORTHWEST). `"0"` gives id 0. Unknown names give `None` (the game returns
    /// 0). Built-in (non-custom) names are not known here.
    pub fn anim_id(&self, name: &str) -> Option<AnimRef> {
        let n = atoi(name);
        if n > 0 {
            return u16::try_from(n).ok().map(|id| AnimRef {
                id,
                direction: None,
            });
        }
        if let Some(a) = self.animation(name) {
            return Some(AnimRef {
                id: a.id,
                direction: None,
            });
        }
        if name == "0" {
            return Some(AnimRef {
                id: 0,
                direction: None,
            });
        }
        let (base, dir) = name.split_once(':')?;
        let a = self.animation(base)?;
        Some(AnimRef {
            id: a.id,
            direction: direction_from_name(dir),
        })
    }

    /// `get_icon_id` (:2791-2801): a non-zero number is the id itself; else a custom icon name.
    pub fn icon_id(&self, name: &str) -> Option<u16> {
        let n = atoi(name);
        if n != 0 {
            return u16::try_from(n).ok();
        }
        if let Some(i) = self.icon(name) {
            return Some(i.id);
        }
        (name == "0").then_some(0)
    }
}
