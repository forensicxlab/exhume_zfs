//! Block reads (DVA selection, checksum verification, decompression) and dnode/objset access.
use crate::blkptr::{BP_SIZE, BlkPtr, VDEV_LABEL_START};
use crate::{checksum, compress, label, source::Source};
use std::io;

pub const DNODE_SIZE: usize = 512;

fn err<T>(m: String) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, m))
}

#[derive(Debug, Clone)]
pub struct Dnode {
    pub ty: u8,
    pub indblkshift: u8,
    pub nlevels: u8,
    pub bonustype: u8,
    pub checksum: u8,
    pub compress: u8,
    pub flags: u8,
    /// Extra 512-byte slots this dnode occupies (large dnodes).
    pub extra_slots: u8,
    pub datablksz: u64,
    pub maxblkid: u64,
    pub used: u64,
    pub blkptrs: Vec<BlkPtr>,
    pub bonus: Vec<u8>,
    /// Spill block pointer (DNODE_FLAG_SPILL_BLKPTR), stored in the last 128 bytes.
    pub spill: Option<BlkPtr>,
}

const DNODE_FLAG_SPILL_BLKPTR: u8 = 0x04;

impl Dnode {
    /// `b` holds the whole dnode: 512 bytes, or 512 * (1 + extra_slots) for a large dnode.
    pub fn parse(b: &[u8]) -> Self {
        let nblkptr = (b[3] as usize).min(3); // a 512-byte dnode holds at most 3 block pointers
        let bonuslen = u16::from_le_bytes([b[10], b[11]]) as usize;
        let extra_slots = b[12];
        let dnsize = (DNODE_SIZE * (1 + extra_slots as usize)).min(b.len());
        let flags = b[7];
        let bp0 = 64;
        let bonus_off = bp0 + nblkptr * BP_SIZE;
        let spill = (flags & DNODE_FLAG_SPILL_BLKPTR != 0 && dnsize >= bp0 + BP_SIZE)
            .then(|| BlkPtr::parse(&b[dnsize - BP_SIZE..dnsize]));
        let bonus_end = if spill.is_some() {
            dnsize - BP_SIZE
        } else {
            dnsize
        };
        Dnode {
            ty: b[0],
            indblkshift: b[1],
            nlevels: b[2],
            bonustype: b[4],
            checksum: b[5],
            compress: b[6],
            flags,
            extra_slots,
            datablksz: (u16::from_le_bytes([b[8], b[9]]) as u64) << 9,
            maxblkid: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            used: u64::from_le_bytes(b[24..32].try_into().unwrap()),
            blkptrs: (0..nblkptr)
                .map(|i| BlkPtr::parse(&b[bp0 + i * BP_SIZE..]))
                .collect(),
            bonus: b[bonus_off.min(bonus_end)..(bonus_off + bonuslen).min(bonus_end)].to_vec(),
            spill,
        }
    }
    pub fn is_free(&self) -> bool {
        self.ty == 0
    }
}

pub struct Pool<S: Source> {
    pub src: S,
    pub config: crate::nvlist::NvList,
    pub uberblock: label::Uberblock,
}

impl<S: Source> Pool<S> {
    pub fn open(src: S) -> io::Result<Self> {
        let l = label::read_best(&src)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no valid ZFS label"))?;
        Ok(Pool {
            src,
            config: l.config,
            uberblock: l.uberblock,
        })
    }

    /// Read, verify and decompress one block; returns its logical content.
    pub fn read_bp(&self, bp: &BlkPtr) -> io::Result<Vec<u8>> {
        if bp.embedded {
            return self.read_embedded(bp);
        }
        if bp.is_hole() {
            return Ok(vec![0u8; bp.lsize as usize]);
        }
        let mut last = String::new();
        for d in bp.dva.iter().filter(|d| d.asize != 0) {
            if d.vdev != 0 {
                last = format!("vdev {} not supported (single disk only)", d.vdev);
                continue;
            }
            if d.gang {
                last = "gang block not supported".into();
                continue;
            }
            let mut phys = vec![0u8; bp.psize as usize];
            if let Err(e) = self.src.read_at(VDEV_LABEL_START + d.offset, &mut phys) {
                last = e.to_string();
                continue;
            }
            match checksum::verify(bp.checksum, &phys, &bp.cksum, bp.little_endian) {
                Some(true) => {}
                Some(false) => {
                    last = format!("checksum mismatch at {:#x}", d.offset);
                    continue;
                }
                None => {} // algorithm not implemented: data is returned unverified
            }
            return if bp.compress == 2 || bp.psize == bp.lsize && bp.compress == 0 {
                phys.truncate(bp.lsize as usize);
                Ok(phys)
            } else {
                compress::decompress(bp.compress, &phys, bp.lsize as usize).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("decompress alg {} failed", bp.compress),
                    )
                })
            };
        }
        err(last)
    }

    fn read_embedded(&self, bp: &BlkPtr) -> io::Result<Vec<u8>> {
        let raw = bp.raw();
        let mut payload = Vec::with_capacity(112);
        for w in (0..16).filter(|w| *w != 6 && *w != 10) {
            payload.extend_from_slice(&raw[w * 8..w * 8 + 8]);
        }
        payload.truncate(bp.psize as usize);
        if bp.compress == 2 || bp.compress == 0 {
            payload.truncate(bp.lsize as usize);
            return Ok(payload);
        }
        compress::decompress(bp.compress, &payload, bp.lsize as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "embedded decompress failed"))
    }

    /// Read data block `blkid` (level 0) of an object, walking the indirect tree.
    pub fn read_block(&self, dn: &Dnode, blkid: u64) -> io::Result<Vec<u8>> {
        if !(7..=24).contains(&dn.indblkshift)
            || !(1..=8).contains(&dn.nlevels)
            || dn.datablksz == 0
        {
            return err(format!(
                "bad dnode geometry (indblkshift {}, nlevels {}, blksz {})",
                dn.indblkshift, dn.nlevels, dn.datablksz
            ));
        }
        let fan = 1u64 << (dn.indblkshift - 7);
        let top = dn.nlevels as u32 - 1;
        let Some(span) = fan.checked_pow(top) else {
            return err("indirect tree too deep".into());
        };
        let n = dn.blkptrs.len() as u64;
        let idx = blkid / span;
        if idx >= n {
            return Ok(vec![0u8; dn.datablksz as usize]);
        }
        let mut bp = dn.blkptrs[idx as usize].clone();
        for level in (0..top).rev() {
            if bp.is_hole() {
                return Ok(vec![0u8; dn.datablksz as usize]);
            }
            let ind = self.read_bp(&bp)?;
            let i = (blkid / fan.pow(level)) % fan;
            let at = i as usize * BP_SIZE;
            let Some(e) = ind.get(at..at + BP_SIZE) else {
                return err("short indirect block".into());
            };
            bp = BlkPtr::parse(e);
        }
        if bp.is_hole() {
            // a hole's blkptr is all zero, so its own lsize says nothing: use the dnode block size
            return Ok(vec![0u8; dn.datablksz as usize]);
        }
        self.read_bp(&bp)
    }

    /// Read a whole (small) object, e.g. a ZAP or an objset array.
    pub fn read_object(&self, dn: &Dnode) -> io::Result<Vec<u8>> {
        const MAX_OBJECT: u64 = 1 << 30;
        if dn.maxblkid.saturating_add(1).saturating_mul(dn.datablksz) > MAX_OBJECT {
            return err(format!("object too large to load ({} blocks)", dn.maxblkid));
        }
        let mut out = Vec::new();
        for b in 0..=dn.maxblkid {
            out.extend(self.read_block(dn, b)?);
        }
        Ok(out)
    }

    /// Dnode `obj` of an object set whose metadnode is `meta`.
    pub fn dnode_at(&self, meta: &Dnode, obj: u64) -> io::Result<Dnode> {
        let per_block = meta.datablksz / DNODE_SIZE as u64;
        if per_block == 0 {
            return err("metadnode block size below one dnode".into());
        }
        let blk = self.read_block(meta, obj / per_block)?;
        let o = (obj % per_block) as usize * DNODE_SIZE;
        if blk.len() < o + DNODE_SIZE {
            return err("short metadnode block".into());
        }
        // a large dnode spans extra slots: hand the parser everything up to the block end
        let extra = blk[o + 12] as usize;
        let end = (o + DNODE_SIZE * (1 + extra)).min(blk.len());
        Ok(Dnode::parse(&blk[o..end]))
    }

    /// The metadnode (first 512 bytes) of the objset pointed to by `bp`.
    pub fn objset_meta(&self, bp: &BlkPtr) -> io::Result<Dnode> {
        let os = self.read_bp(bp)?;
        match os.get(..DNODE_SIZE) {
            Some(d) => Ok(Dnode::parse(d)),
            None => err("objset block shorter than a dnode".into()),
        }
    }
}
