//! Sprite sheets: `X.dat` (RLE pixel data) + `X.tab` (6-byte records: u32 offset, u8 w, u8 h).
//! Reference `spritesheet.cpp:20-82` (`load_index_file`, `load_data_file`).
use crate::image::Rgba;
use crate::install::Install;
use crate::palette::Palette;
use crate::rle;
use crate::{Error, Result};

pub use crate::gen_sprite_names::{GBS_BUTTON_SPRITES, GFS_FRONTEND_SPRITES, GPS_PANEL_SPRITES};

/// Custom icons get ids from here up (`GUI_PANEL_SPRITES_COUNT`, src/sprites.h:779).
pub const GUI_PANEL_SPRITES_COUNT: u16 = 950;
/// Number of built-in `gui1-64` button sprite ids (`GUI_BUTTON_SPRITES_COUNT`, src/sprites.h:213).
pub const GUI_BUTTON_SPRITES_COUNT: u16 = 245;

/// Index of a named `gui2-64` panel sprite (`GPS_*` constant of sprites.h, e.g.
/// `GPS_portrt_qmark`).
pub fn panel_sprite_index(name: &str) -> Option<u16> {
    GPS_PANEL_SPRITES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, v)| v)
}

/// Name of a `gui2-64` sprite index, if the C enum names it.
pub fn panel_sprite_name(index: u16) -> Option<&'static str> {
    GPS_PANEL_SPRITES
        .iter()
        .find(|(_, v)| *v == index)
        .map(|&(n, _)| n)
}

/// Index of a named `gui1-64` button sprite (`GBS_*`).
pub fn button_sprite_index(name: &str) -> Option<u16> {
    GBS_BUTTON_SPRITES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, v)| v)
}

/// Name of a `gui1-64` sprite index.
pub fn button_sprite_name(index: u16) -> Option<&'static str> {
    GBS_BUTTON_SPRITES
        .iter()
        .find(|(_, v)| *v == index)
        .map(|&(n, _)| n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetEntry {
    pub offset: u32,
    pub w: u8,
    pub h: u8,
}

pub struct SpriteSheet {
    entries: Vec<SheetEntry>,
    data: Vec<u8>,
}

impl SpriteSheet {
    pub fn from_bytes(tab: &[u8], dat: Vec<u8>) -> Result<Self> {
        let n = tab.len() / 6;
        if n == 0 {
            return Err(Error::format("sheet", "empty .tab"));
        }
        let entries = tab
            .chunks_exact(6)
            .map(|b| SheetEntry {
                offset: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                w: b[4],
                h: b[5],
            })
            .collect();
        Ok(SpriteSheet { entries, data: dat })
    }

    /// Load `data/<stem>.dat` and `data/<stem>.tab` (e.g. `gui2-64`, `gui1-64`, `pointer64`).
    pub fn load(install: &Install, stem: &str) -> Result<Self> {
        let tab = install.load(&format!("data/{stem}.tab"))?;
        let dat = install.load(&format!("data/{stem}.dat"))?;
        Self::from_bytes(&tab, dat)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, i: usize) -> Option<&SheetEntry> {
        self.entries.get(i)
    }

    pub fn entries(&self) -> &[SheetEntry] {
        &self.entries
    }

    /// Data slice of entry `i` (to the next larger offset, else the file end).
    pub fn data_range(&self, i: usize) -> Option<(usize, usize)> {
        let e = self.entries.get(i)?;
        let s = e.offset as usize;
        let end = rle::entry_end(
            self.entries.iter().map(|x| x.offset),
            e.offset,
            self.data.len(),
        );
        if s > self.data.len() || s > end {
            return None;
        }
        Some((s, end))
    }

    pub fn decode_indexed(&self, i: usize) -> Result<(rle::Indexed, rle::RleStats)> {
        let e = *self
            .entries
            .get(i)
            .ok_or_else(|| Error::format("sheet", format!("index {i} out of range")))?;
        if e.w == 0 || e.h == 0 {
            return Ok((
                rle::Indexed {
                    w: e.w as usize,
                    h: e.h as usize,
                    ..Default::default()
                },
                Default::default(),
            ));
        }
        let (s, en) = self.data_range(i).ok_or_else(|| {
            Error::format(
                "sheet",
                format!("entry {i}: offset {} outside data", e.offset),
            )
        })?;
        rle::decode(&self.data[s..en], e.w as usize, e.h as usize)
            .map_err(|er| Error::format("sheet", format!("entry {i}: {er}")))
    }

    /// Decode entry `i` to RGBA (`w` x `h`). Zero-sized entries give an empty image.
    pub fn decode(&self, i: usize, pal: &Palette) -> Result<Rgba> {
        Ok(self.decode_indexed(i)?.0.to_rgba(pal))
    }
}
