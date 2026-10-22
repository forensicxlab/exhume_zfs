use exhume_zfs::blkptr::BlkPtr;
use exhume_zfs::{checksum, compress, nvlist, zap};

const TEXT: &[u8] = b"hello hello hello hello hello hello\n";

#[test]
fn fletcher4_known_vector() {
    // words 1 and 2: a=1,b=1,c=1,d=1 then a=3,b=4,c=5,d=6
    let data = [1u8, 0, 0, 0, 2, 0, 0, 0];
    assert_eq!(checksum::fletcher4(&data, true), [3, 4, 5, 6]);
    let be = [0u8, 0, 0, 1, 0, 0, 0, 2];
    assert_eq!(checksum::fletcher4(&be, false), [3, 4, 5, 6]);
}

#[test]
fn fletcher2_known_vector() {
    let mut d = Vec::new();
    d.extend(1u64.to_le_bytes());
    d.extend(2u64.to_le_bytes());
    d.extend(3u64.to_le_bytes());
    d.extend(4u64.to_le_bytes());
    // a0=1+3, a1=2+4, b0=1+4, b1=2+6
    assert_eq!(checksum::fletcher2(&d, true), [4, 6, 5, 8]);
}

#[test]
fn sha256_abc() {
    assert_eq!(checksum::sha256(b"abc")[0], 0xba78_16bf_8f01_cfea);
    assert_eq!(
        checksum::verify(8, b"abc", &checksum::sha256(b"abc"), true),
        Some(true)
    );
    assert_eq!(
        checksum::verify(8, b"abd", &checksum::sha256(b"abc"), true),
        Some(false)
    );
    assert_eq!(checksum::verify(99, b"x", &[0; 4], true), None);
}

#[test]
fn lz4_block_with_zfs_length_prefix() {
    let body = [
        0x6f, b'h', b'e', b'l', b'l', b'o', b' ', 0x06, 0x00, 0x06, 0x50, b'e', b'l', b'l', b'o',
        b'\n',
    ];
    let mut blk = (body.len() as u32).to_be_bytes().to_vec();
    blk.extend(body);
    assert_eq!(compress::decompress(15, &blk, TEXT.len()).unwrap(), TEXT);
    assert_eq!(
        compress::decompress(1, &blk, TEXT.len()).unwrap(),
        TEXT,
        "'on' means lz4"
    );
    // truncated input and a wrong size must fail, not panic
    assert!(compress::decompress(15, &blk[..8], TEXT.len()).is_none());
    assert!(compress::decompress(15, &blk, TEXT.len() + 1).is_none());
}

#[test]
fn zstd_with_zfs_header() {
    // frame produced by the zstd CLI for TEXT, wrapped as ZFS does: BE length, 4 bytes version/level, frame
    let frame: [u8; 26] = [
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x58, 0x6d, 0x00, 0x00, 0x38, 0x68, 0x65, 0x6c, 0x6c, 0x6f,
        0x20, 0x0a, 0x01, 0x00, 0xb9, 0x4b, 0x11, 0x6b, 0x1f, 0x08, 0x67,
    ];
    // ZFS writes magicless frames (ZSTD_f_zstd1_magicless): drop the CLI's 4-byte magic
    let body = &frame[4..];
    let mut blk = (body.len() as u32).to_be_bytes().to_vec();
    blk.extend([0, 0, 0, 3]);
    blk.extend(body);
    assert_eq!(compress::decompress(16, &blk, TEXT.len()).unwrap(), TEXT);
    assert!(compress::decompress(16, &blk[..12], TEXT.len()).is_none());
    assert!(
        compress::decompress(16, &blk, TEXT.len() + 1).is_none(),
        "size must match"
    );
    // a frame that still carries the magic is not what ZFS stores
    let mut with_magic = (frame.len() as u32).to_be_bytes().to_vec();
    with_magic.extend([0, 0, 0, 3]);
    with_magic.extend(frame);
    assert!(compress::decompress(16, &with_magic, TEXT.len()).is_none());
}

#[test]
fn zle_and_lzjb_and_gzip() {
    // zle: literal run of 3 ("abc") then 5 zeros (64 + 4)
    let z = [2, b'a', b'b', b'c', 64 + 4];
    assert_eq!(compress::decompress(14, &z, 8).unwrap(), b"abc\0\0\0\0\0");
    // lzjb: 8 literals under a zero copymap, then one copy (offset 8, length 3+0)
    let mut j = vec![0b0000_0000u8];
    j.extend(b"abcdefgh");
    j.push(0b0000_0001);
    j.extend([0b0000_0000, 8]);
    assert_eq!(compress::decompress(3, &j, 11).unwrap(), b"abcdefghabc");
    assert!(
        compress::decompress(3, &[0b1, 0, 9], 11).is_none(),
        "offset beyond output"
    );
    // gzip level 6 is a zlib stream
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    e.write_all(TEXT).unwrap();
    assert_eq!(
        compress::decompress(10, &e.finish().unwrap(), TEXT.len()).unwrap(),
        TEXT
    );
    assert_eq!(compress::decompress(2, TEXT, 5).unwrap(), &TEXT[..5], "off");
    assert_eq!(
        compress::decompress(4, &[], 4).unwrap(),
        vec![0; 4],
        "empty"
    );
    assert!(compress::decompress(200, TEXT, 5).is_none());
}

fn xdr_str(v: &mut Vec<u8>, s: &str) {
    v.extend((s.len() as u32).to_be_bytes());
    v.extend(s.as_bytes());
    v.extend(std::iter::repeat_n(0u8, (4 - s.len() % 4) % 4));
}

fn pair(name: &str, ty: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    xdr_str(&mut body, name);
    body.extend(ty.to_be_bytes());
    body.extend(1u32.to_be_bytes());
    body.extend(payload);
    let mut p = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    p.extend((body.len() as u32).to_be_bytes());
    p.extend(body);
    p
}

#[test]
fn nvlist_decodes_scalars_and_nested() {
    let mut inner = vec![0, 0, 0, 0, 0, 0, 0, 0];
    inner.extend(pair("ashift", 8, &12u64.to_be_bytes()));
    inner.extend([0u8; 8]);
    let mut s = Vec::new();
    xdr_str(&mut s, "rpool");
    let mut buf = vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    buf.extend(pair("name", 9, &s));
    buf.extend(pair("vdev_tree", 19, &inner));
    buf.extend(pair("flag", 21, &1u32.to_be_bytes()));
    buf.extend([0u8; 8]);
    let nv = nvlist::decode(&buf).unwrap();
    assert_eq!(nv["name"], nvlist::Value::Str("rpool".into()));
    assert_eq!(nv["flag"], nvlist::Value::Bool(true));
    let nvlist::Value::List(t) = &nv["vdev_tree"] else {
        panic!()
    };
    assert_eq!(t["ashift"], nvlist::Value::U64(12));
}

#[test]
fn nvlist_rejects_garbage_without_panicking() {
    assert!(nvlist::decode(&[]).is_none());
    assert!(nvlist::decode(&[0, 1, 0, 0]).is_none());
    assert!(
        nvlist::decode(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]).is_none()
    );
}

#[test]
fn blkptr_fields() {
    let mut b = [0u8; 128];
    // DVA0: vdev 0, asize 8 sectors, offset 0x1000 sectors
    b[0..8].copy_from_slice(&8u64.to_le_bytes());
    b[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
    // prop: lsize 4096 (7), psize 2048 (3), compress lz4 (15), checksum fletcher4 (7), type 19, level 1, LE
    let prop = 7u64 | 3 << 16 | 15 << 32 | 7 << 40 | 19 << 48 | 1 << 56 | 1 << 63;
    b[48..56].copy_from_slice(&prop.to_le_bytes());
    b[80..88].copy_from_slice(&42u64.to_le_bytes());
    b[88..96].copy_from_slice(&3u64.to_le_bytes());
    b[96..104].copy_from_slice(&9u64.to_le_bytes());
    let bp = BlkPtr::parse(&b);
    assert_eq!(
        (bp.dva[0].asize, bp.dva[0].offset, bp.dva[0].gang),
        (4096, 0x20_0000, false)
    );
    assert_eq!(
        (
            bp.lsize,
            bp.psize,
            bp.compress,
            bp.checksum,
            bp.ty,
            bp.level
        ),
        (4096, 2048, 15, 7, 19, 1)
    );
    assert!(bp.little_endian && !bp.embedded && !bp.is_hole());
    assert_eq!((bp.birth, bp.fill, bp.cksum[0]), (42, 3, 9));
    assert!(BlkPtr::parse(&[0u8; 128]).is_hole());
}

#[test]
fn microzap_entries() {
    let mut blk = vec![0u8; 512];
    blk[..8].copy_from_slice(&((1u64 << 63) + 3).to_le_bytes());
    // entry 0: value 0x1234, name "ROOT"
    blk[64..72].copy_from_slice(&0x1234u64.to_le_bytes());
    blk[64 + 14..64 + 18].copy_from_slice(b"ROOT");
    let e = zap::parse(&blk, 512);
    assert_eq!(e.len(), 1);
    assert_eq!((e[0].name.as_str(), e[0].as_u64()), ("ROOT", Some(0x1234)));
}

#[test]
fn packed_ids_and_permissions() {
    use exhume_zfs::fs::{pack_id, unpack_id};
    let id = pack_id(3, 12931).unwrap();
    assert_eq!(unpack_id(id), (3, 12931));
    assert!(pack_id(1 << 16, 1).is_none() && pack_id(1, 1 << 48).is_none());
    let a = exhume_zfs::zpl::Attrs {
        mode: 0o104755,
        ..Default::default()
    };
    assert_eq!(a.permissions(), "-rwxr-xr-x");
    let d = exhume_zfs::zpl::Attrs {
        mode: 0o040700,
        ..Default::default()
    };
    assert_eq!(d.permissions(), "drwx------");
}

#[test]
fn large_dnode_bonus_and_spill() {
    use exhume_zfs::pool::Dnode;
    let mut b = vec![0u8; 1024];
    b[0] = 19; // plain file
    b[3] = 1; // one block pointer
    b[7] = 0x04; // spill pointer present
    b[10..12].copy_from_slice(&600u16.to_le_bytes()); // bonus larger than a 512-byte dnode allows
    b[12] = 1; // one extra slot
    b[64 + 128] = 0xAB; // first bonus byte
    b[1024 - 128] = 8; // spill DVA asize word
    let dn = Dnode::parse(&b);
    assert_eq!((dn.extra_slots, dn.bonus.len()), (1, 600));
    assert_eq!(dn.bonus[0], 0xAB);
    assert!(dn.spill.is_some());
    // the same bytes seen as a 512-byte dnode: the spill pointer then sits in the
    // last 128 bytes of that slot and the bonus is clipped in front of it
    let small = Dnode::parse(&b[..512]);
    assert_eq!(small.bonus.len(), 512 - 64 - 128 - 128);
    assert!(small.spill.is_some());
}
