//! The sprite RLE shared by keeper sprites (`creature.jty`) and sprite sheets (`X.dat`).
//!
//! Per pixel row: a signed byte `cmd`; `cmd < 0` skips `-cmd` transparent pixels, `cmd > 0` is
//! followed by `cmd` literal palette indices (index 0 is opaque too), `cmd == 0` ends the row.
//! Reference: `GLWorldViewRenderer.cpp:84-126` (`decode_keeper_rle`) and `engine_render.c`.
use crate::image::Rgba;
use crate::palette::Palette;
use crate::{Error, Result};

/// Decoded, still palettised sprite: `idx[i]` is meaningful only where `mask[i] != 0`.
#[derive(Clone, Debug, Default)]
pub struct Indexed {
    pub w: usize,
    pub h: usize,
    pub idx: Vec<u8>,
    pub mask: Vec<u8>,
}

/// Oddities met while decoding that the game tolerates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RleStats {
    /// Pixels the stream wanted to draw at `x >= w` (the game drops them).
    pub overflow_px: usize,
    /// Bytes of the slice left unread after the last row.
    pub trailing: usize,
}

impl Indexed {
    pub fn to_rgba(&self, pal: &Palette) -> Rgba {
        let mut out = Rgba::new(self.w as u32, self.h as u32);
        for i in 0..self.w * self.h {
            if self.mask[i] != 0 {
                out.data[i * 4..i * 4 + 4].copy_from_slice(&pal.rgba(self.idx[i]));
            }
        }
        out
    }
}

/// Decode `h` rows of `w` pixels from `data`. Errors when the data ends before `h` rows are done.
pub fn decode(data: &[u8], w: usize, h: usize) -> Result<(Indexed, RleStats)> {
    let mut out = Indexed {
        w,
        h,
        idx: vec![0; w * h],
        mask: vec![0; w * h],
    };
    let mut st = RleStats::default();
    let mut p = 0usize;
    for y in 0..h {
        let mut x = 0usize;
        loop {
            let Some(&b) = data.get(p) else {
                return Err(Error::format(
                    "rle",
                    format!("data ended inside row {y} of {h}"),
                ));
            };
            p += 1;
            let cmd = b as i8;
            if cmd == 0 {
                break;
            }
            if cmd < 0 {
                x += (-(cmd as i32)) as usize;
            } else {
                for _ in 0..cmd as usize {
                    let Some(&v) = data.get(p) else {
                        return Err(Error::format(
                            "rle",
                            format!("literal run past end in row {y}"),
                        ));
                    };
                    p += 1;
                    if x < w {
                        out.idx[y * w + x] = v;
                        out.mask[y * w + x] = 255;
                    } else {
                        st.overflow_px += 1;
                    }
                    x += 1;
                }
            }
        }
    }
    st.trailing = data.len() - p;
    Ok((out, st))
}

/// Byte range `[offset, end)` of the entry at `offset`: up to the next strictly larger offset in
/// `all_offsets`, else `file_len`. Mirrors how the game cuts sheet data (`spritesheet.cpp:51-72`).
pub fn entry_end(all_offsets: impl Iterator<Item = u32>, offset: u32, file_len: usize) -> usize {
    all_offsets
        .filter(|&o| o > offset)
        .min()
        .map_or(file_len, |o| (o as usize).min(file_len))
}
