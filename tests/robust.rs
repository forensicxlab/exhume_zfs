//! Random and mutated input must produce errors, never panics.
use exhume_zfs::blkptr::BlkPtr;
use exhume_zfs::pool::{Dnode, Pool};
use exhume_zfs::source::Source;
use exhume_zfs::{compress, label, nvlist, zap};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

struct Mem(Vec<u8>);
impl Source for Mem {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> std::io::Result<()> {
        let o = off as usize;
        match self.0.get(o..o + buf.len()) {
            Some(s) => {
                buf.copy_from_slice(s);
                Ok(())
            }
            None => Err(std::io::ErrorKind::UnexpectedEof.into()),
        }
    }
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
}

#[test]
fn random_buffers_into_every_decoder() {
    let mut r = Rng(0x1234_5678_9abc_def1);
    for i in 0..4000 {
        let n = [0usize, 3, 12, 64, 512, 4096, 20000][i % 7];
        let b = r.bytes(n);
        let _ = nvlist::decode(&b);
        let _ = zap::parse(&b, 512);
        let _ = zap::parse(&b, 4096);
        if b.len() >= 128 {
            let bp = BlkPtr::parse(&b);
            let _ = bp.is_hole();
        }
        if b.len() >= 512 {
            let _ = Dnode::parse(&b[..512]);
        }
        for alg in 0..20u8 {
            let _ = compress::decompress(alg, &b, 1 << (i % 14));
        }
    }
}

#[test]
fn leaf_like_and_fat_zap_with_random_content() {
    let mut r = Rng(77);
    for _ in 0..500 {
        let mut b = r.bytes(2 * 16384);
        b[..8].copy_from_slice(&((1u64 << 63) + 1).to_le_bytes());
        b[16384..16384 + 8].copy_from_slice(&(1u64 << 63).to_le_bytes());
        b[16384 + 24..16384 + 28].copy_from_slice(&0x02AB_1EAF_u32.to_le_bytes());
        for i in 0..400 {
            let o = 16384 + 48 + 1024 + i * 24;
            if o + 24 <= b.len() && i % 3 == 0 {
                b[o] = 252; // entry chunks pointing at random chunks
            }
        }
        let _ = zap::parse(&b, 16384);
    }
}

#[test]
fn mutated_labels_do_not_panic() {
    let mut r = Rng(99);
    // a plausible label region: valid-looking nvlist header, magic-bearing uberblocks
    let mut base = vec![0u8; 1 << 20];
    base[16384..16388].copy_from_slice(&[1, 1, 0, 0]);
    for s in 0..32 {
        let o = 131072 + s * 4096;
        base[o..o + 8].copy_from_slice(&0x00ba_b10cu64.to_le_bytes());
        base[o + 16..o + 24].copy_from_slice(&(s as u64 + 1).to_le_bytes());
    }
    for _ in 0..2000 {
        let mut m = base.clone();
        for _ in 0..(1 + r.next() % 40) {
            let at = (r.next() as usize) % 262144;
            m[at] = r.next() as u8;
        }
        let _ = label::read_best(&Mem(m.clone()));
        let _ = Pool::open(Mem(m));
    }
}

#[test]
fn dnode_with_hostile_fields() {
    let mut r = Rng(5);
    for _ in 0..3000 {
        let mut b = r.bytes(512);
        b[3] = (r.next() % 4) as u8 * 85; // 0, 85, 170, 255 block pointers
        b[1] = (r.next() % 40) as u8; // indblkshift, including < 7
        b[2] = (r.next() % 8) as u8; // nlevels, including 0
        let dn = Dnode::parse(&b);
        let pool = Pool::open(Mem(vec![0u8; 8])).err();
        let _ = pool;
        // read_block on a pool without data: must be an error or zeros, never a panic
        let p = fake_pool();
        let _ = p.read_block(&dn, r.next() % 1000);
        let _ = p.dnode_at(&dn, r.next() % 1000);
    }
}

fn fake_pool() -> Pool<Mem> {
    // 1 MiB of zeros with a valid empty label so Pool::open succeeds
    let mut m = vec![0u8; 8 << 20];
    m[16384..16388].copy_from_slice(&[1, 1, 0, 0]);
    m[16388..16396].copy_from_slice(&[0; 8]);
    let o = 131072;
    m[o..o + 8].copy_from_slice(&0x00ba_b10cu64.to_le_bytes());
    Pool::open(Mem(m)).expect("synthetic pool")
}
