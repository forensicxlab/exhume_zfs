//! Block decompressors: off, lzjb, zle, lz4, gzip (zlib), zstd.
use std::io::Read;

pub fn decompress(alg: u8, src: &[u8], lsize: usize) -> Option<Vec<u8>> {
    match alg {
        2 => Some(src[..lsize.min(src.len())].to_vec()), // off
        1 | 15 => lz4(src, lsize),                       // "on" resolves to lz4
        3 => lzjb(src, lsize),
        4 => Some(vec![0u8; lsize]), // empty
        14 => zle(src, lsize),
        5..=13 => {
            let mut out = Vec::with_capacity(lsize);
            flate2::read::ZlibDecoder::new(src)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        16 => zstd(src, lsize),
        _ => None,
    }
}

fn lz4(src: &[u8], lsize: usize) -> Option<Vec<u8>> {
    let n = u32::from_be_bytes(src.get(..4)?.try_into().ok()?) as usize;
    let s = src.get(4..4 + n)?;
    let mut out = Vec::with_capacity(lsize);
    let mut i = 0;
    while i < s.len() {
        let t = s[i] as usize;
        i += 1;
        let mut lit = t >> 4;
        if lit == 15 {
            loop {
                let b = *s.get(i)? as usize;
                i += 1;
                lit += b;
                if b != 255 {
                    break;
                }
            }
        }
        if out.len() + lit > lsize {
            return None;
        }
        out.extend_from_slice(s.get(i..i + lit)?);
        i += lit;
        if i >= s.len() {
            break;
        }
        let off = u16::from_le_bytes(s.get(i..i + 2)?.try_into().ok()?) as usize;
        i += 2;
        let mut ml = t & 15;
        if ml == 15 {
            loop {
                let b = *s.get(i)? as usize;
                i += 1;
                ml += b;
                if b != 255 {
                    break;
                }
            }
        }
        ml += 4;
        if off == 0 || off > out.len() || out.len() + ml > lsize {
            return None;
        }
        for _ in 0..ml {
            out.push(out[out.len() - off]);
        }
    }
    (out.len() == lsize).then_some(out)
}

fn lzjb(src: &[u8], lsize: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(lsize);
    let (mut i, mut copymask, mut copymap) = (0, 128u32, 0u8);
    while out.len() < lsize {
        copymask <<= 1;
        if copymask == 256 {
            copymask = 1;
            copymap = *src.get(i)?;
            i += 1;
        }
        if copymap as u32 & copymask != 0 {
            let (b0, b1) = (*src.get(i)? as usize, *src.get(i + 1)? as usize);
            i += 2;
            let ml = (b0 >> 2) + 3;
            let off = ((b0 & 3) << 8) | b1;
            if off == 0 || off > out.len() {
                return None;
            }
            for _ in 0..ml {
                if out.len() >= lsize {
                    break;
                }
                out.push(out[out.len() - off]);
            }
        } else {
            out.push(*src.get(i)?);
            i += 1;
        }
    }
    Some(out)
}

fn zle(src: &[u8], lsize: usize) -> Option<Vec<u8>> {
    const N: usize = 64; // zle level used by ZFS
    let mut out = Vec::with_capacity(lsize);
    let mut i = 0;
    while out.len() < lsize {
        let b = *src.get(i)? as usize;
        i += 1;
        if b >= N {
            let len = b - N + 1;
            out.extend(std::iter::repeat_n(0u8, len));
        } else {
            let len = b + 1;
            out.extend_from_slice(src.get(i..i + len)?);
            i += len;
        }
    }
    out.truncate(lsize);
    Some(out)
}

fn zstd(src: &[u8], lsize: usize) -> Option<Vec<u8>> {
    // zfs_zstdhdr_t: 4 bytes BE compressed length, 4 bytes version/level, then a
    // frame written with ZSTD_f_zstd1_magicless: the 4-byte magic is not stored.
    const MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
    let n = u32::from_be_bytes(src.get(..4)?.try_into().ok()?) as usize;
    let body = src.get(8..8 + n)?;
    let mut frame = Vec::with_capacity(4 + n);
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(body);
    let mut dec = ruzstd::decoding::StreamingDecoder::new(frame.as_slice()).ok()?;
    let mut out = Vec::with_capacity(lsize);
    dec.read_to_end(&mut out).ok()?;
    (out.len() == lsize).then_some(out)
}
