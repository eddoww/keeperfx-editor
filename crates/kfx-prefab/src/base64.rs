//! Minimal base64 (standard alphabet, padded) for the embedded thumbnail.
const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(ALPHA[(n >> 18) as usize & 63] as char);
        out.push(ALPHA[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            ALPHA[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            ALPHA[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Decode; `None` on characters outside the alphabet or a bad length. White space is ignored.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let mut vals = vec![];
    let mut pad = 0;
    for b in s.bytes().filter(|b| !b.is_ascii_whitespace()) {
        if b == b'=' {
            pad += 1;
            continue;
        }
        if pad > 0 {
            return None;
        }
        vals.push(ALPHA.iter().position(|&a| a == b)? as u32);
    }
    if (vals.len() + pad) % 4 != 0 || pad > 2 {
        return None;
    }
    let mut out = vec![];
    for c in vals.chunks(4) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, v)| a | v << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        for n in 0..40usize {
            let d: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(super::decode(&super::encode(&d)).unwrap(), d);
        }
        assert_eq!(super::encode(b"Man"), "TWFu");
        assert!(super::decode("a").is_none());
    }
}
