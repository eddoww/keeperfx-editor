//! The 256-colour game palette.
use crate::install::Install;
use crate::{Error, Result};

/// How a 6-bit channel is widened to 8 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// `v * 255 / 63`, the display palette (`RendererManager.h:51` `RendererPaletteChannel8`,
    /// used by `RendererPaletteSet`, RendererManager.cpp:109-120). Default.
    Display,
    /// `v << 2`, what the tile atlas packer does (`TileAtlasPacker.cpp:92-94`); max channel is 252.
    Shl2,
}

/// 256 colours of 6-bit RGB (`data/palette.dat`, 768 bytes, values 0..=63; VGA).
#[derive(Clone, Debug)]
pub struct Palette {
    raw: [u8; 768],
    scale: Scale,
}

impl Palette {
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        Self::from_bytes_scaled(b, Scale::Display)
    }

    pub fn from_bytes_scaled(b: &[u8], scale: Scale) -> Result<Self> {
        if b.len() < 768 {
            return Err(Error::format(
                "palette",
                format!("{} bytes, need 768", b.len()),
            ));
        }
        let mut raw = [0u8; 768];
        raw.copy_from_slice(&b[..768]);
        Ok(Palette { raw, scale })
    }

    /// Load `data/palette.dat` of the install.
    pub fn load(install: &Install) -> Result<Self> {
        Self::from_bytes(&install.load("data/palette.dat")?)
    }

    /// Raw 6-bit channel values as stored.
    pub fn raw(&self) -> &[u8; 768] {
        &self.raw
    }

    /// Highest raw channel value (a valid VGA palette is <= 63).
    pub fn max_raw(&self) -> u8 {
        self.raw.iter().copied().max().unwrap_or(0)
    }

    fn ch(&self, v: u8) -> u8 {
        match self.scale {
            Scale::Display => ((v as u32 * 255) / 63).min(255) as u8,
            Scale::Shl2 => v << 2,
        }
    }

    pub fn rgb(&self, i: u8) -> [u8; 3] {
        let o = i as usize * 3;
        [
            self.ch(self.raw[o]),
            self.ch(self.raw[o + 1]),
            self.ch(self.raw[o + 2]),
        ]
    }

    /// Opaque colour of palette entry `i`.
    pub fn rgba(&self, i: u8) -> [u8; 4] {
        let c = self.rgb(i);
        [c[0], c[1], c[2], 255]
    }

    /// All 256 entries as opaque RGBA (for an egui/wgpu palette view).
    pub fn table(&self) -> [[u8; 4]; 256] {
        let mut t = [[0; 4]; 256];
        for (i, e) in t.iter_mut().enumerate() {
            *e = self.rgba(i as u8);
        }
        t
    }
}
