//! RNC ("Rob Northen Compression") method 1 unpacking. Port of KeeperFX `src/bflib_dernc.c`
//! (`rnc_unpack`, bflib_dernc.c:~100-250), in strict mode (flags = 0): any CRC or range error
//! fails.
//!
//! Header (18 bytes, `RNC_HEADER_LEN`, src/bflib_dernc.h:53): magic `RNC\x01`, then big-endian
//! u32 unpacked size, u32 packed size, u16 unpacked CRC, u16 packed CRC, and one unused u16.
use crate::{Error, Result};
use std::path::Path;

const HEADER_LEN: usize = 18;

/// True when `data` starts with the RNC method-1 magic.
pub fn is_rnc(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"RNC\x01"
}

fn err(msg: impl Into<String>) -> Error {
    Error::format("rnc", msg)
}

/// The RNC CRC-16 (polynomial 0xA001, reflected), bflib_dernc.c `rnc_crc`.
pub fn crc(data: &[u8]) -> u16 {
    let mut tab = [0u16; 256];
    for (i, t) in tab.iter_mut().enumerate() {
        let mut v = i as u16;
        for _ in 0..8 {
            v = if v & 1 != 0 {
                (v >> 1) ^ 0xA001
            } else {
                v >> 1
            };
        }
        *t = v;
    }
    let mut v: u16 = 0;
    for &b in data {
        v ^= b as u16;
        v = (v >> 8) ^ tab[(v & 0xFF) as usize];
    }
    v
}

#[derive(Default, Clone, Copy)]
struct Entry {
    code: u64,
    len: u32,
    value: u32,
}

#[derive(Default)]
struct Huf {
    n: usize,
    t: [Entry; 32],
}

/// Bit reader over the packed body. `pos` is the byte cursor (`*p` in the C), which may run
/// past `end`; reads beyond the buffer contribute zero bits, like the C range checks.
struct Bits<'a> {
    buf: &'a [u8], // body only; end == buf.len()
    pos: usize,
    bitbuf: u64,
    bitcount: i32,
}

impl<'a> Bits<'a> {
    fn lword(&self) -> u64 {
        let g = |i: usize| self.buf.get(i).copied().unwrap_or(0) as u64;
        (g(self.pos + 1) << 8) | g(self.pos)
    }
    fn new(buf: &'a [u8]) -> Self {
        let mut b = Bits {
            buf,
            pos: 0,
            bitbuf: 0,
            bitcount: 16,
        };
        b.bitbuf = b.lword();
        b
    }
    fn fix(&mut self) {
        // `bitcount` is 16..=31 here (see `advance`)
        self.bitcount = (self.bitcount - 16).clamp(0, 47);
        self.bitbuf &= 1u64
            .checked_shl(self.bitcount as u32)
            .unwrap_or(0)
            .wrapping_sub(1);
        if self.pos <= self.buf.len() {
            self.bitbuf |= self.lword().checked_shl(self.bitcount as u32).unwrap_or(0);
        }
        self.bitcount += 16;
    }
    fn peek(&self, mask: u64) -> u64 {
        self.bitbuf & mask
    }
    /// Drop `n` bits. A well-formed stream never asks for more bits than are buffered (at
    /// least 16); a malformed one can (a Huffman leaf value up to 31 reads `value - 1` bits),
    /// where the C shifts by a negative count: that is an error here.
    fn advance(&mut self, n: i32) -> Result<()> {
        if n < 0 || n > self.bitcount {
            return Err(err("bit read beyond the buffered bits (malformed stream)"));
        }
        self.bitbuf = self.bitbuf.checked_shr(n as u32).unwrap_or(0);
        self.bitcount -= n;
        if self.bitcount < 16 {
            self.pos += 2;
            if self.pos <= self.buf.len() {
                self.bitbuf |= self.lword().checked_shl(self.bitcount as u32).unwrap_or(0);
            }
            self.bitcount += 16;
        }
        Ok(())
    }
    fn read(&mut self, mask: u64, n: i32) -> Result<u64> {
        let r = self.peek(mask);
        self.advance(n)?;
        Ok(r)
    }
}

fn mirror(mut x: u64, n: u32) -> u64 {
    let mut top = 1u64 << (n - 1);
    let mut bottom = 1u64;
    while top > bottom {
        let mask = top | bottom;
        let masked = x & mask;
        if masked != 0 && masked != mask {
            x ^= mask;
        }
        top >>= 1;
        bottom <<= 1;
    }
    x
}

fn read_huftable(bs: &mut Bits) -> Result<Huf> {
    let mut h = Huf::default();
    let num = bs.read(0x1F, 5)? as usize;
    if num == 0 {
        return Ok(h);
    }
    let mut leaflen = [0u32; 32];
    let mut leafmax = 1;
    for l in leaflen.iter_mut().take(num) {
        *l = bs.read(0x0F, 4)? as u32;
        leafmax = leafmax.max(*l);
    }
    let mut codeb = 0u64;
    let mut k = 0;
    for i in 1..=leafmax {
        for (j, &l) in leaflen.iter().enumerate().take(num) {
            if l == i {
                h.t[k] = Entry {
                    code: mirror(codeb, i),
                    len: i,
                    value: j as u32,
                };
                codeb += 1;
                k += 1;
            }
        }
        codeb <<= 1;
    }
    h.n = k;
    Ok(h)
}

/// `Ok(None)` is the C's `-1` (no matching code).
fn huf_read(h: &Huf, bs: &mut Bits) -> Result<Option<u64>> {
    let Some(i) = (0..h.n).find(|&i| bs.peek((1u64 << h.t[i].len) - 1) == h.t[i].code) else {
        return Ok(None);
    };
    let e = h.t[i];
    bs.advance(e.len as i32)?;
    let mut val = e.value as u64;
    if val >= 2 {
        val = 1u64 << (val - 1);
        val |= bs.read(val - 1, e.value as i32 - 1)?;
    }
    Ok(Some(val))
}

/// Unpack an RNC method-1 buffer. Verifies both CRCs and the unpacked size.
pub fn unpack(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < HEADER_LEN || !is_rnc(data) {
        return Err(err("bad or missing RNC header"));
    }
    let be32 = |o: usize| u32::from_be_bytes(data[o..o + 4].try_into().unwrap());
    let be16 = |o: usize| u16::from_be_bytes(data[o..o + 2].try_into().unwrap());
    let unpacked_size = be32(4) as usize;
    let packed_size = be32(8) as usize;
    let unpacked_crc = be16(12);
    let packed_crc = be16(14);
    if unpacked_size > 1 << 30 || packed_size > 1 << 30 {
        return Err(err("header sizes out of range"));
    }
    // The C reads packed_size bytes past the header without checking the file length.
    let body = data
        .get(HEADER_LEN..HEADER_LEN + packed_size)
        .ok_or_else(|| err("packed data truncated"))?;
    if crc(body) != packed_crc {
        return Err(err("packed data CRC mismatch"));
    }
    let mut out = vec![0u8; unpacked_size];
    let mut o = 0usize;
    let mut bs = Bits::new(body);
    bs.advance(2)?;
    let exceeds = || err("data exceeds range");
    let bad_code = || err("Huffman decode error");

    while o < unpacked_size {
        if (body.len() as i64) - (bs.pos as i64) < 6 {
            return Err(exceeds());
        }
        let raw = read_huftable(&mut bs)?;
        let dist = read_huftable(&mut bs)?;
        let len = read_huftable(&mut bs)?;
        let mut ch_count = bs.read(0xFFFF, 16)?;
        loop {
            let mut length = huf_read(&raw, &mut bs)?.ok_or_else(bad_code)?;
            if length > 0 {
                while length > 0 {
                    length -= 1;
                    if bs.pos >= body.len() || o >= unpacked_size {
                        return Err(exceeds());
                    }
                    out[o] = body[bs.pos];
                    o += 1;
                    bs.pos += 1;
                }
                bs.fix();
            }
            ch_count = ch_count.wrapping_sub(1);
            if ch_count == 0 {
                break;
            }
            let posn = huf_read(&dist, &mut bs)?.ok_or_else(bad_code)? as usize + 1;
            let mut length = huf_read(&len, &mut bs)?.ok_or_else(bad_code)? + 2;
            while length > 0 {
                length -= 1;
                if posn > o || o >= unpacked_size {
                    return Err(exceeds());
                }
                out[o] = out[o - posn];
                o += 1;
            }
        }
    }
    if crc(&out) != unpacked_crc {
        return Err(err("unpacked data CRC mismatch"));
    }
    Ok(out)
}

/// Read a file, transparently unpacking it when it is RNC-packed.
pub fn load_file(path: &Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    if is_rnc(&data) {
        unpack(&data).map_err(|e| match e {
            Error::Format { what, msg } => Error::Format {
                what,
                msg: format!("{}: {msg}", path.display()),
            },
            o => o,
        })
    } else {
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::install_dir;

    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(unpack(b"RNC\x01").is_err());
        assert!(unpack(b"nope nope nope nope nope").is_err());
        let mut h = b"RNC\x01".to_vec();
        h.extend_from_slice(&[0, 0, 0, 10, 0, 0, 0, 200, 0, 0, 0, 0, 0, 0]);
        assert!(unpack(&h).is_err());
    }

    /// LSB-first bit writer matching [`Bits`] (16-bit little-endian words).
    struct BitWriter {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }
    impl BitWriter {
        fn put(&mut self, v: u64, bits: u32) {
            self.acc |= (v & ((1 << bits) - 1)) << self.n;
            self.n += bits;
            while self.n >= 8 {
                self.out.push(self.acc as u8);
                self.acc >>= 8;
                self.n -= 8;
            }
        }
    }

    /// A stream whose raw Huffman table has one code standing for value 30: decoding it asks the
    /// bit reader for 29 bits at once, more than it holds. The C shifts by a negative count there
    /// (`bs->bitcount` below zero); the port panicked with a shift overflow in debug builds and
    /// produced wrong bits in release builds (fuzz input `rnc-181.bin`).
    #[test]
    fn oversized_bit_read_is_an_error() {
        let mut w = BitWriter {
            out: vec![],
            acc: 0,
            n: 0,
        };
        w.put(0, 2); // skipped by the decoder
        w.put(31, 5); // raw table: 31 leaves
        for j in 0..31 {
            w.put(if j == 30 { 1 } else { 0 }, 4);
        }
        w.put(0, 5); // empty distance table
        w.put(0, 5); // empty length table
        w.put(1, 16); // one chunk
        w.put(0, 1); // the code of leaf 30
        for _ in 0..64 {
            w.put(0, 8);
        }
        let body = w.out;
        let mut data = b"RNC\x01".to_vec();
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&crc(&body).to_be_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&body);
        let r = std::panic::catch_unwind(|| unpack(&data));
        assert!(
            matches!(r, Ok(Err(_))),
            "must be an error, not a panic or data: {:?}",
            r.as_ref().map(|x| x.as_ref().map(|v| v.len()))
        );
    }

    #[test]
    fn install_rnc_files_unpack() {
        let Some(dir) = install_dir() else { return };
        let mut files = Vec::new();
        walk(&dir, &mut files);
        let (mut n, mut bad) = (0, 0);
        for p in files {
            let Ok(mut f) = std::fs::File::open(&p) else {
                continue;
            };
            let mut magic = [0u8; 4];
            use std::io::Read;
            if f.read_exact(&mut magic).is_err() || &magic != b"RNC\x01" {
                continue;
            }
            let data = std::fs::read(&p).unwrap();
            let want = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
            match unpack(&data) {
                Ok(v) if v.len() == want => n += 1,
                Ok(v) => {
                    bad += 1;
                    println!("RNC size mismatch {}: {} vs {}", p.display(), v.len(), want);
                }
                Err(e) => {
                    bad += 1;
                    println!("RNC FAIL {}: {e}", p.display());
                }
            }
        }
        println!("RNC files unpacked OK: {n}, failures: {bad}");
        assert_eq!(bad, 0);
    }
}
