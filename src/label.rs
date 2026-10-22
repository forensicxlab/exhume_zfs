use crate::nvlist::{self, NvList};
use crate::source::Source;

pub const LABEL_SIZE: u64 = 256 * 1024;
const NVLIST_OFF: u64 = 16 * 1024;
const NVLIST_LEN: usize = 112 * 1024;
const UB_RING_OFF: u64 = 128 * 1024;
const UB_RING_LEN: usize = 128 * 1024;
const UB_MAGIC: u64 = 0x00ba_b10c;

#[derive(Debug, Clone)]
pub struct Uberblock {
    pub version: u64,
    pub txg: u64,
    pub guid_sum: u64,
    pub timestamp: u64,
    pub rootbp: [u8; 128],
    pub slot: usize,
    pub label: usize,
}

pub struct Label {
    pub config: NvList,
    pub uberblock: Uberblock,
}

fn label_offsets(len: u64) -> [u64; 4] {
    let usable = len & !(LABEL_SIZE - 1);
    [0, LABEL_SIZE, usable - 2 * LABEL_SIZE, usable - LABEL_SIZE]
}

/// Read the four labels and return the config of the first valid one plus the
/// valid uberblock with the highest txg (ties: latest timestamp).
pub fn read_best(src: &dyn Source) -> Option<Label> {
    if src.len() < 4 * LABEL_SIZE {
        return None; // too small to hold the four labels
    }
    let mut best: Option<Uberblock> = None;
    let mut config = None;
    for (li, off) in label_offsets(src.len()).iter().enumerate() {
        let mut nv = vec![0u8; NVLIST_LEN];
        if config.is_none() && src.read_at(off + NVLIST_OFF, &mut nv).is_ok() {
            config = nvlist::decode(&nv);
        }
        let mut ring = vec![0u8; UB_RING_LEN];
        if src.read_at(off + UB_RING_OFF, &mut ring).is_err() {
            continue;
        }
        let slot_size = uberblock_slot_size(&config);
        for (slot, u) in ring.chunks(slot_size).enumerate() {
            let w = |i: usize| u64::from_le_bytes(u[i * 8..i * 8 + 8].try_into().unwrap());
            if u.len() < 168 || w(0) != UB_MAGIC {
                continue;
            }
            let mut rootbp = [0u8; 128];
            rootbp.copy_from_slice(&u[40..168]);
            let ub = Uberblock {
                version: w(1),
                txg: w(2),
                guid_sum: w(3),
                timestamp: w(4),
                rootbp,
                slot,
                label: li,
            };
            if best
                .as_ref()
                .is_none_or(|b| (ub.txg, ub.timestamp) > (b.txg, b.timestamp))
            {
                best = Some(ub);
            }
        }
    }
    Some(Label {
        config: config?,
        uberblock: best?,
    })
}

/// Uberblock slot size: 1 KiB minimum, otherwise 1 << ashift, capped at 8 KiB.
fn uberblock_slot_size(config: &Option<NvList>) -> usize {
    use crate::nvlist::Value;
    let ashift = match config.as_ref().and_then(|c| c.get("vdev_tree")) {
        Some(Value::List(t)) => match t.get("ashift") {
            Some(Value::U64(a)) => *a as u32,
            _ => 9,
        },
        _ => 9,
    };
    1usize << ashift.clamp(10, 13)
}
