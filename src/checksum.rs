//! Block checksums. Values are compared as four u64 words in the byte order of the block pointer.
use sha2::{Digest, Sha256};

pub const CKSUM_FLETCHER2: u8 = 6;
pub const CKSUM_FLETCHER4: u8 = 7;
pub const CKSUM_SHA256: u8 = 8;
pub const CKSUM_OFF: u8 = 2;

pub fn fletcher4(data: &[u8], le: bool) -> [u64; 4] {
    let (mut a, mut b, mut c, mut d) = (0u64, 0u64, 0u64, 0u64);
    for ch in data.chunks_exact(4) {
        let x = if le {
            u32::from_le_bytes(ch.try_into().unwrap())
        } else {
            u32::from_be_bytes(ch.try_into().unwrap())
        } as u64;
        a = a.wrapping_add(x);
        b = b.wrapping_add(a);
        c = c.wrapping_add(b);
        d = d.wrapping_add(c);
    }
    [a, b, c, d]
}

pub fn fletcher2(data: &[u8], le: bool) -> [u64; 4] {
    let (mut a0, mut a1, mut b0, mut b1) = (0u64, 0u64, 0u64, 0u64);
    for ch in data.chunks_exact(16) {
        let rd = |o: usize| {
            let x: [u8; 8] = ch[o..o + 8].try_into().unwrap();
            if le {
                u64::from_le_bytes(x)
            } else {
                u64::from_be_bytes(x)
            }
        };
        a0 = a0.wrapping_add(rd(0));
        a1 = a1.wrapping_add(rd(8));
        b0 = b0.wrapping_add(a0);
        b1 = b1.wrapping_add(a1);
    }
    [a0, a1, b0, b1]
}

pub fn sha256(data: &[u8]) -> [u64; 4] {
    let h = Sha256::digest(data);
    let mut o = [0u64; 4];
    for (i, w) in o.iter_mut().enumerate() {
        *w = u64::from_be_bytes(h[i * 8..i * 8 + 8].try_into().unwrap());
    }
    o
}

/// Returns Some(ok) for supported algorithms, None for the ones we cannot verify.
pub fn verify(alg: u8, data: &[u8], expected: &[u64; 4], le: bool) -> Option<bool> {
    match alg {
        CKSUM_OFF => Some(true),
        CKSUM_FLETCHER2 => Some(&fletcher2(data, le) == expected),
        CKSUM_FLETCHER4 | 1 => Some(&fletcher4(data, le) == expected), // 1 = "on" resolves to fletcher4
        CKSUM_SHA256 => Some(&sha256(data) == expected),
        _ => None,
    }
}
