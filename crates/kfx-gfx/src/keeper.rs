//! Keeper sprites: creature/thing animations from `data/creature.tab` + `data/creature.jty`.
//!
//! `creature.tab` is an array of 16-byte `KeeperSpriteDisk` records (`src/creature_graphics.h:92-104`,
//! unpacked by `src/creature_graphics.c:613-636`); `creature.jty` holds the RLE pixel data
//! ([`crate::rle`]) at `DataOffset`. Animation ids index [`CREATURE_LIST`] to get the first tab
//! index of the animation (`src/creature_graphics.c:226-238` `keepersprite_index`).
use crate::gen_creature_list::CREATURE_LIST;
use crate::image::Rgba;
use crate::install::Install;
use crate::palette::Palette;
use crate::rle;
use crate::{Error, Result};

pub use crate::gen_creature_list::CREATURE_LIST as ANIMATION_TABLE;

/// Ids from this value up are custom animations (`src/engine_render.h:39`).
pub const KEEPERSPRITE_ADD_OFFSET: u16 = 16384;
/// Number of built-in animation ids (`CREATURE_FRAMELIST_LENGTH`, src/creature_graphics.h:30).
pub const ANIMATION_COUNT: usize = CREATURE_LIST.len();

/// One `creature.tab` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabEntry {
    pub data_offset: u32,
    /// Pixel size of the stored (cropped) bitmap.
    pub s_width: u8,
    pub s_height: u8,
    /// Size of the full animation frame box the bitmap sits in.
    pub frame_width: u8,
    pub frame_height: u8,
    /// 0 = single view, 2 = five stored views (the rest are mirrors). 1 is not drawn by the game.
    pub rotable: u8,
    pub frames_count: u8,
    /// Position of the bitmap inside the frame box.
    pub frame_offs_w: u8,
    pub frame_offs_h: u8,
    /// Frame box origin relative to the thing's anchor point.
    pub offset_x: i16,
    pub offset_y: i16,
}

impl TabEntry {
    const SIZE: usize = 16;

    fn parse(b: &[u8]) -> TabEntry {
        TabEntry {
            data_offset: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            s_width: b[4],
            s_height: b[5],
            frame_width: b[6],
            frame_height: b[7],
            rotable: b[8],
            frames_count: b[9],
            frame_offs_w: b[10],
            frame_offs_h: b[11],
            offset_x: i16::from_le_bytes([b[12], b[13]]),
            offset_y: i16::from_le_bytes([b[14], b[15]]),
        }
    }
}

/// A decoded frame with the data needed to place it.
#[derive(Clone, Debug)]
pub struct KeeperFrame {
    pub image: Rgba,
    pub entry: TabEntry,
    pub stats: rle::RleStats,
}

/// An animation: `frames` frames, in `directions` views (1 or 5), starting at tab index `first`.
/// Tab layout: for each view, `frames` consecutive entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animation {
    pub id: u16,
    pub first_frame: usize,
    pub frames: u8,
    pub rotable: u8,
    /// 5 when `rotable == 2`, else 1.
    pub directions: u8,
}

/// Which stored view and whether to mirror it for a facing angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    /// Stored view number 0..=4.
    pub quarter: u8,
    pub flip: bool,
}

/// Facing angle (0..2047 = 360 degrees, `ANGLE_MASK` 2047) to stored view + mirror, for
/// `rotable == 2` sprites. `src/engine_render.c:7598-7600` (quarter) and :7648-7651 (mirror when the
/// angle is in 1152..=1918; note the game's test makes 1919 unmirrored despite quarter 3).
pub fn view_for_angle(angle: i32) -> View {
    let a = angle & 2047;
    let i = (a + 128) & 2047;
    let quarter = (4 - (i >> 8)).unsigned_abs() as u8;
    let flip = !(a <= 1151 || a >= 1919);
    View { quarter, flip }
}

impl Animation {
    /// Tab index of `frame` (clamped to the last frame like the game, :7517-7519) in `quarter`.
    /// Non-rotable animations ignore `quarter`.
    pub fn tab_index(&self, frame: usize, quarter: u8) -> usize {
        let f = frame.min(self.frames.saturating_sub(1) as usize);
        let q = if self.rotable == 2 {
            quarter.min(4) as usize
        } else {
            0
        };
        self.first_frame + f + q * self.frames as usize
    }

    /// Pick the tab index and mirror flag for a facing angle (`rotable == 2` only mirrors).
    pub fn pick(&self, angle: i32, frame: usize) -> (usize, bool) {
        if self.rotable == 2 {
            let v = view_for_angle(angle);
            (self.tab_index(frame, v.quarter), v.flip)
        } else {
            (self.tab_index(frame, 0), false)
        }
    }
}

/// Loaded `creature.tab` + `creature.jty`.
pub struct KeeperSprites {
    tab: Vec<TabEntry>,
    jty: Vec<u8>,
}

impl KeeperSprites {
    pub fn from_bytes(tab: &[u8], jty: Vec<u8>) -> Result<Self> {
        if tab.len() % TabEntry::SIZE != 0 {
            return Err(Error::format(
                "creature.tab",
                format!("size {} not a multiple of 16", tab.len()),
            ));
        }
        let tab = tab
            .chunks_exact(TabEntry::SIZE)
            .map(TabEntry::parse)
            .collect();
        Ok(KeeperSprites { tab, jty })
    }

    pub fn load(install: &Install) -> Result<Self> {
        Self::from_bytes(
            &install.load("data/creature.tab")?,
            install.load("data/creature.jty")?,
        )
    }

    /// Number of tab entries (individual frames of all views of all animations).
    pub fn len(&self) -> usize {
        self.tab.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tab.is_empty()
    }

    pub fn entry(&self, i: usize) -> Option<&TabEntry> {
        self.tab.get(i)
    }

    pub fn entries(&self) -> &[TabEntry] {
        &self.tab
    }

    /// Byte range of frame `i` in the jty: up to the next entry's offset
    /// (`src/engine_render.c:7527` `load_single_frame`), the last one to the file end.
    pub fn data_range(&self, i: usize) -> Option<(usize, usize)> {
        let s = self.tab.get(i)?.data_offset as usize;
        let e = self
            .tab
            .get(i + 1)
            .map_or(self.jty.len(), |n| n.data_offset as usize);
        if s > e || e > self.jty.len() {
            return None;
        }
        Some((s, e))
    }

    /// Palettised bitmap of tab entry `i`.
    pub fn decode_indexed(&self, i: usize) -> Result<(rle::Indexed, rle::RleStats)> {
        let en = *self
            .tab
            .get(i)
            .ok_or_else(|| Error::format("keeper", format!("tab index {i} out of range")))?;
        let (s, e) = self.data_range(i).ok_or_else(|| {
            Error::format(
                "keeper",
                format!("frame {i}: data range outside creature.jty"),
            )
        })?;
        rle::decode(&self.jty[s..e], en.s_width as usize, en.s_height as usize)
            .map_err(|er| Error::format("keeper", format!("frame {i}: {er}")))
    }

    /// Decode tab entry `i` to RGBA (`s_width` x `s_height`, transparent where the RLE skips).
    pub fn decode(&self, i: usize, pal: &Palette) -> Result<KeeperFrame> {
        let (ix, stats) = self.decode_indexed(i)?;
        Ok(KeeperFrame {
            image: ix.to_rgba(pal),
            entry: self.tab[i],
            stats,
        })
    }

    /// Animation for a built-in id (< 982). `None` for custom ids (>= 16384, see [`crate::custom`])
    /// or when the id's tab entry does not exist.
    pub fn animation(&self, anim_id: u16) -> Option<Animation> {
        let first = *CREATURE_LIST.get(anim_id as usize)? as usize;
        let e = self.tab.get(first)?;
        Some(Animation {
            id: anim_id,
            first_frame: first,
            frames: e.frames_count,
            rotable: e.rotable,
            directions: if e.rotable == 2 { 5 } else { 1 },
        })
    }

    /// Top-left of the bitmap relative to the thing's anchor, and its size, for tab entry `i` drawn
    /// as part of `anim`. Frame box origin comes from the animation's first entry
    /// (`src/engine_render.c:7653-7670`), the bitmap position inside the box from entry `i`. With
    /// `flip` the placement is mirrored around the anchor (the bitmap must be mirrored too, see
    /// [`Rgba::flipped_x`]).
    pub fn placement(&self, anim: &Animation, i: usize, flip: bool) -> Option<(i32, i32)> {
        let first = self.tab.get(anim.first_frame)?;
        let e = self.tab.get(i)?;
        let y = first.offset_y as i32 + e.frame_offs_h as i32;
        let x = if flip {
            -(first.offset_x as i32 + e.frame_offs_w as i32 + e.s_width as i32)
        } else {
            first.offset_x as i32 + e.frame_offs_w as i32
        };
        Some((x, y))
    }
}
