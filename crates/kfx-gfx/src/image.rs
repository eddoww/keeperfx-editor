//! A plain RGBA8 image.
use crate::{Error, Result};

/// Straight (non-premultiplied) RGBA8 image, row-major, `w * h * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Rgba {
    /// Fully transparent image.
    pub fn new(w: u32, h: u32) -> Self {
        Rgba {
            w,
            h,
            data: vec![0; w as usize * h as usize * 4],
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let o = (y as usize * self.w as usize + x as usize) * 4;
        [
            self.data[o],
            self.data[o + 1],
            self.data[o + 2],
            self.data[o + 3],
        ]
    }

    pub fn put(&mut self, x: u32, y: u32, px: [u8; 4]) {
        if x < self.w && y < self.h {
            let o = (y as usize * self.w as usize + x as usize) * 4;
            self.data[o..o + 4].copy_from_slice(&px);
        }
    }

    /// Mirror left-right (used for the mirrored rotable creature views).
    pub fn flipped_x(&self) -> Rgba {
        let mut out = Rgba::new(self.w, self.h);
        for y in 0..self.h {
            for x in 0..self.w {
                out.put(self.w - 1 - x, y, self.pixel(x, y));
            }
        }
        out
    }

    /// Copy `src` onto `self` at `(x, y)` (may be negative); alpha-over, clipped.
    pub fn blit(&mut self, src: &Rgba, x: i32, y: i32) {
        for sy in 0..src.h as i32 {
            for sx in 0..src.w as i32 {
                let (dx, dy) = (x + sx, y + sy);
                if dx < 0 || dy < 0 || dx >= self.w as i32 || dy >= self.h as i32 {
                    continue;
                }
                let s = src.pixel(sx as u32, sy as u32);
                match s[3] {
                    0 => {}
                    255 => self.put(dx as u32, dy as u32, s),
                    a => {
                        let d = self.pixel(dx as u32, dy as u32);
                        let mix = |sc: u8, dc: u8| {
                            ((sc as u32 * a as u32 + dc as u32 * (255 - a as u32)) / 255) as u8
                        };
                        let da = (a as u32 + d[3] as u32 * (255 - a as u32) / 255) as u8;
                        self.put(
                            dx as u32,
                            dy as u32,
                            [mix(s[0], d[0]), mix(s[1], d[1]), mix(s[2], d[2]), da],
                        );
                    }
                }
            }
        }
    }

    /// Sub-rectangle copy; the rectangle must lie inside the image.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Option<Rgba> {
        if x.checked_add(w)? > self.w || y.checked_add(h)? > self.h {
            return None;
        }
        let mut out = Rgba::new(w, h);
        for yy in 0..h {
            let s = ((y + yy) as usize * self.w as usize + x as usize) * 4;
            let d = yy as usize * w as usize * 4;
            out.data[d..d + w as usize * 4].copy_from_slice(&self.data[s..s + w as usize * 4]);
        }
        Some(out)
    }

    /// Encode as a PNG file image.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, self.w.max(1), self.h.max(1));
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc
                .write_header()
                .map_err(|e| Error::format("png", e.to_string()))?;
            if self.w == 0 || self.h == 0 {
                wr.write_image_data(&[0, 0, 0, 0])
                    .map_err(|e| Error::format("png", e.to_string()))?;
            } else {
                wr.write_image_data(&self.data)
                    .map_err(|e| Error::format("png", e.to_string()))?;
            }
        }
        Ok(buf)
    }

    /// Decode any 8-bit PNG (palette, grey, RGB, with or without alpha / tRNS) into RGBA.
    /// Like the game (`src/custom_sprites.c:759-763`) only 8-bit depth is accepted.
    pub fn from_png(bytes: &[u8]) -> Result<Rgba> {
        let perr = |e: png::DecodingError| Error::format("png", e.to_string());
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND);
        let mut rd = dec.read_info().map_err(perr)?;
        let (ct, depth) = rd.output_color_type();
        if depth != png::BitDepth::Eight {
            return Err(Error::format(
                "png",
                format!("bit depth {depth:?}, expected 8"),
            ));
        }
        let mut buf = vec![0; rd.output_buffer_size()];
        let info = rd.next_frame(&mut buf).map_err(perr)?;
        let (w, h) = (info.width, info.height);
        let n = w as usize * h as usize;
        let src = &buf[..info.buffer_size()];
        let mut data = Vec::with_capacity(n * 4);
        match ct {
            png::ColorType::Rgba => data.extend_from_slice(&src[..n * 4]),
            png::ColorType::Rgb => src[..n * 3]
                .chunks(3)
                .for_each(|p| data.extend_from_slice(&[p[0], p[1], p[2], 255])),
            png::ColorType::GrayscaleAlpha => src[..n * 2]
                .chunks(2)
                .for_each(|p| data.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
            png::ColorType::Grayscale => src[..n]
                .iter()
                .for_each(|&g| data.extend_from_slice(&[g, g, g, 255])),
            png::ColorType::Indexed => return Err(Error::format("png", "palette not expanded")),
        }
        Ok(Rgba { w, h, data })
    }
}
