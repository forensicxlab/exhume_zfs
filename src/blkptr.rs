//! Block pointer (blkptr_t, 128 bytes) decoding.
pub const BP_SIZE: usize = 128;
/// Data starts after the two front labels and the boot area.
pub const VDEV_LABEL_START: u64 = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dva {
    pub vdev: u32,
    pub offset: u64, // bytes, relative to the end of the front labels
    pub asize: u64,  // bytes
    pub gang: bool,
}

#[derive(Debug, Clone)]
pub struct BlkPtr {
    pub dva: [Dva; 3],
    pub lsize: u64,
    pub psize: u64,
    pub compress: u8,
    pub embedded: bool,
    pub checksum: u8,
    pub ty: u8,
    pub level: u8,
    pub little_endian: bool,
    pub birth: u64,
    pub fill: u64,
    pub cksum: [u64; 4],
    raw: [u8; BP_SIZE],
}

fn w(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap())
}

impl BlkPtr {
    pub fn parse(b: &[u8]) -> Self {
        let mut raw = [0u8; BP_SIZE];
        raw.copy_from_slice(&b[..BP_SIZE]);
        let prop = w(b, 6);
        let embedded = (prop >> 39) & 1 == 1;
        let mut dva = [Dva {
            vdev: 0,
            offset: 0,
            asize: 0,
            gang: false,
        }; 3];
        for (i, d) in dva.iter_mut().enumerate() {
            let (w0, w1) = (w(b, i * 2), w(b, i * 2 + 1));
            *d = Dva {
                vdev: (w0 >> 32) as u32,
                asize: (w0 & 0xff_ffff) << 9,
                offset: (w1 & 0x7fff_ffff_ffff_ffff) << 9,
                gang: w1 >> 63 == 1,
            };
        }
        let (lsize, psize) = if embedded {
            (((prop & 0x1ff_ffff) + 1), (((prop >> 25) & 0x7f) + 1))
        } else {
            (
                (((prop & 0xffff) + 1) << 9),
                ((((prop >> 16) & 0xffff) + 1) << 9),
            )
        };
        BlkPtr {
            dva,
            lsize,
            psize,
            compress: ((prop >> 32) & 0x7f) as u8,
            embedded,
            checksum: if embedded {
                0
            } else {
                ((prop >> 40) & 0xff) as u8
            },
            ty: ((prop >> 48) & 0xff) as u8,
            level: ((prop >> 56) & 0x1f) as u8,
            little_endian: prop >> 63 == 1,
            birth: w(b, 10),
            fill: w(b, 11),
            cksum: [w(b, 12), w(b, 13), w(b, 14), w(b, 15)],
            raw,
        }
    }

    /// A hole has no valid DVA and no data.
    pub fn is_hole(&self) -> bool {
        !self.embedded && self.dva[0].asize == 0 && self.birth == 0
    }

    pub fn raw(&self) -> &[u8; BP_SIZE] {
        &self.raw
    }
}
