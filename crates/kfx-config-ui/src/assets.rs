//! Art the editors show, handed in by the host (already loaded; nothing here reads files
//! except the optional [`Assets::load_install`] convenience).
use crate::fptd::td_animation;
use crate::raster::TileCache;
use kfx_gfx::custom::KEEPERSPRITE_ADD_OFFSET;
use kfx_gfx::sheet::GUI_PANEL_SPRITES_COUNT;
use kfx_gfx::textures::{TextureAnim, TextureSearch, TextureSet};
use kfx_gfx::{CustomSet, Install, KeeperSprites, Palette, Rgba, SpriteSheet};
use kfx_map::gamedata::parse_number;
use std::path::Path;

/// Display art: palette, texture atlas (with animation table), creature / thing sprites, GUI
/// sheets, custom sprite zips.
pub struct Assets {
    pub palette: Palette,
    pub tiles: Option<TileCache>,
    pub keeper: Option<KeeperSprites>,
    /// `gui2-64`: panel sprites (room / spell / trap symbols).
    pub gui2: Option<SpriteSheet>,
    /// `gui1-64`: button sprites, creature portraits (`QuerySymbol`).
    pub gui1: Option<SpriteSheet>,
    pub custom: CustomSet,
}

/// One decoded animation frame and where its bitmap sits relative to the anchor.
#[derive(Clone, Debug)]
pub struct Frame {
    pub image: Rgba,
    pub x: i32,
    pub y: i32,
}

impl Assets {
    /// Everything from an install (texture set 0, `data/` only), plus the zips of `extra_zip_dirs`
    /// (campaign config folder ...) and `extra_zips` (e.g. `mapNNNNN.zip`), in that order.
    pub fn load_install(
        install: &Path,
        search: Option<TextureSearch>,
        level: Option<u32>,
        extra_zip_dirs: &[&Path],
        extra_zips: &[&Path],
    ) -> Result<Assets, String> {
        let inst = Install::new(install);
        let palette = Palette::load(&inst).map_err(|e| format!("palette: {e}"))?;
        let anim = TextureAnim::load(&inst).unwrap_or_default();
        let search = search.unwrap_or_else(|| TextureSearch::new(inst.data_dir()));
        let tiles = TextureSet::load(&search, 0, level)
            .ok()
            .map(|s| TileCache::new(palette.clone(), s, anim));
        let mut custom = CustomSet::new();
        custom.load_fxdata(&inst.fxdata_dir());
        for d in extra_zip_dirs {
            custom.load_dir(d);
        }
        for z in extra_zips {
            let _ = custom.load_zip(z);
        }
        Ok(Assets {
            keeper: KeeperSprites::load(&inst).ok(),
            gui2: SpriteSheet::load(&inst, "gui2-64").ok(),
            gui1: SpriteSheet::load(&inst, "gui1-64").ok(),
            palette,
            tiles,
            custom,
        })
    }

    /// Frames of an animation value as configs write it (number or custom name), top-down view,
    /// direction `quarter` (0 = front .. 4 = back).
    pub fn anim_frames(&self, value: &str, quarter: u8) -> Vec<Frame> {
        let v = value
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('"');
        let Some(r) = self.custom.anim_id(v) else {
            return vec![];
        };
        if r.id == 0 {
            return vec![];
        }
        if r.id >= KEEPERSPRITE_ADD_OFFSET {
            let Some(a) = self
                .custom
                .animation_by_id(r.id)
                .or_else(|| self.custom.animation(v))
            else {
                return vec![];
            };
            let dir = a.td.get(quarter as usize).or(a.td.first());
            let first =
                a.td.first()
                    .and_then(|d| d.first())
                    .and_then(|&id| self.custom.frame(id));
            return dir
                .into_iter()
                .flatten()
                .filter_map(|&id| {
                    let f = self.custom.frame(id)?;
                    let o = first.unwrap_or(f);
                    Some(Frame {
                        image: f.image.clone(),
                        x: o.offset_x + f.offs_w,
                        y: o.offset_y + f.offs_h,
                    })
                })
                .collect();
        }
        let Some(k) = &self.keeper else { return vec![] };
        let Some(a) = k.animation(td_animation(r.id)) else {
            return vec![];
        };
        (0..a.frames as usize)
            .filter_map(|f| {
                let i = a.tab_index(f, quarter);
                let fr = k.decode(i, &self.palette).ok()?;
                let (x, y) = k.placement(&a, i, false)?;
                Some(Frame {
                    image: fr.image,
                    x,
                    y,
                })
            })
            .collect()
    }

    /// An icon value (sprite number of `sheet`, or a custom icon name / number >= 950).
    pub fn icon(&self, value: &str, sheet: Option<&SpriteSheet>) -> Option<Rgba> {
        let v = value.split_whitespace().next()?.trim_matches('"');
        if let Some(n) = parse_number(v) {
            if n >= GUI_PANEL_SPRITES_COUNT as i64 {
                return self.custom.icon_image(n as u16).cloned();
            }
            if n <= 0 {
                return None;
            }
            let img = sheet?.decode(n as usize, &self.palette).ok()?;
            let visible = img.data.chunks_exact(4).any(|p| p[3] > 0);
            return (img.w > 0 && img.h > 0 && visible).then_some(img);
        }
        let i = self.custom.icon(v)?;
        self.custom.icon_image(i.id).cloned()
    }

    /// Creature portrait from `QuerySymbol` (gui1-64).
    pub fn portrait(&self, query_symbol: &str) -> Option<Rgba> {
        self.icon(query_symbol, self.gui1.as_ref())
    }

    /// Panel symbol (gui2-64), e.g. `SymbolSprites` of rooms, spells, traps.
    pub fn panel(&self, value: &str) -> Option<Rgba> {
        self.icon(value, self.gui2.as_ref())
    }
}

/// Compose animation frames on one canvas so they share an anchor (bounding box of all
/// frames). Returns the frame images and the anchor position inside them.
pub fn compose_frames(frames: &[Frame]) -> (Vec<Rgba>, (i32, i32)) {
    if frames.is_empty() {
        return (vec![], (0, 0));
    }
    let x0 = frames.iter().map(|f| f.x).min().unwrap();
    let y0 = frames.iter().map(|f| f.y).min().unwrap();
    let x1 = frames.iter().map(|f| f.x + f.image.w as i32).max().unwrap();
    let y1 = frames.iter().map(|f| f.y + f.image.h as i32).max().unwrap();
    let (w, h) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
    let out = frames
        .iter()
        .map(|f| {
            let mut c = Rgba::new(w, h);
            c.blit(&f.image, f.x - x0, f.y - y0);
            c
        })
        .collect();
    (out, (-x0, -y0))
}
