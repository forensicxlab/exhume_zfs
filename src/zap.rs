//! ZAP objects (microzap and fatzap). Entries are enumerated by scanning leaf blocks directly,
//! so the pointer table and the hash are not needed.
const MICRO: u64 = (1 << 63) + 3;
const LEAF_MAGIC: u32 = 0x02AB_1EAF;
const BLOCK_LEAF: u64 = 1 << 63;
const CH_ENTRY: u8 = 252;
const CH_ARRAY: u8 = 251;
const LEAF_HDR: usize = 48;
const CHUNK: usize = 24;

#[derive(Debug, Clone)]
pub struct ZapEntry {
    pub name: String,
    pub int_size: u8,
    pub data: Vec<u8>,
}

impl ZapEntry {
    /// First integer of the value (big-endian in a fatzap, native in a microzap: both already normalised).
    pub fn as_u64(&self) -> Option<u64> {
        let n = self.int_size as usize;
        if n == 0 || n > 8 || self.data.len() < n {
            return None;
        }
        Some(
            self.data[..n]
                .iter()
                .fold(0u64, |a, b| (a << 8) | *b as u64),
        )
    }
    pub fn as_str(&self) -> String {
        String::from_utf8_lossy(self.data.split(|b| *b == 0).next().unwrap_or(&[])).into_owned()
    }
}

/// `blocks` is the whole object; `bs` its block size.
pub fn parse(blocks: &[u8], bs: usize) -> Vec<ZapEntry> {
    let ty = |b: &[u8]| u64::from_le_bytes(b[..8].try_into().unwrap());
    if blocks.len() < 64 {
        return vec![];
    }
    if ty(blocks) == MICRO {
        return blocks[64..bs.min(blocks.len())]
            .chunks_exact(64)
            .filter(|c| c[14] != 0)
            .map(|c| ZapEntry {
                name: String::from_utf8_lossy(c[14..64].split(|b| *b == 0).next().unwrap())
                    .into_owned(),
                int_size: 8,
                data: c[..8].iter().rev().copied().collect(), // stored LE, normalised to BE order
            })
            .collect();
    }
    let mut out = Vec::new();
    for leaf in blocks.chunks_exact(bs).skip(1) {
        if ty(leaf) != BLOCK_LEAF
            || u32::from_le_bytes(leaf[24..28].try_into().unwrap()) != LEAF_MAGIC
        {
            continue;
        }
        let hash_bytes = bs / 16;
        let chunks = &leaf[LEAF_HDR + hash_bytes..];
        let nchunks = chunks.len() / CHUNK;
        let u16at =
            |c: &[u8], o: usize| u16::from_le_bytes(c[o..o + 2].try_into().unwrap()) as usize;
        let array = |mut ci: usize, len: usize| -> Vec<u8> {
            let mut v = Vec::with_capacity(len);
            let mut guard = 0;
            while ci < nchunks && v.len() < len && guard < nchunks {
                let c = &chunks[ci * CHUNK..(ci + 1) * CHUNK];
                if c[0] != CH_ARRAY {
                    break;
                }
                v.extend_from_slice(&c[1..22]);
                ci = u16at(c, 22);
                guard += 1;
            }
            v.truncate(len);
            v
        };
        for i in 0..nchunks {
            let c = &chunks[i * CHUNK..(i + 1) * CHUNK];
            if c[0] != CH_ENTRY {
                continue;
            }
            let (int_size, name_chunk, name_len, val_chunk, val_n) =
                (c[1], u16at(c, 4), u16at(c, 6), u16at(c, 8), u16at(c, 10));
            let name = array(name_chunk, name_len);
            let data = array(val_chunk, val_n * int_size as usize);
            out.push(ZapEntry {
                name: String::from_utf8_lossy(name.split(|b| *b == 0).next().unwrap_or(&[]))
                    .into_owned(),
                int_size,
                data,
            });
        }
    }
    out
}
