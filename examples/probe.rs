use exhume_zfs::{blkptr::BlkPtr, pool::Pool, source::FileSource};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = FileSource::open(
        &a[1],
        a[2].parse().unwrap(),
        a.get(3).map(|x| x.parse().unwrap()),
    )
    .unwrap();
    let p = Pool::open(src).unwrap();
    let u = &p.uberblock;
    println!("txg {} slot {} version {}", u.txg, u.slot, u.version);
    let bp = BlkPtr::parse(&u.rootbp);
    println!("rootbp {:?}", bp);
    let meta = p.objset_meta(&bp).unwrap();
    println!(
        "MOS metadnode {:?}",
        (meta.ty, meta.nlevels, meta.datablksz, meta.maxblkid)
    );
    for i in 0..70 {
        let d = p.dnode_at(&meta, i).unwrap();
        if !d.is_free() {
            println!(
                "obj {i}: type {} bonus {} len {} blksz {}",
                d.ty,
                d.bonustype,
                d.bonus.len(),
                d.datablksz
            );
        }
    }
}
