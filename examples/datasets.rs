use exhume_zfs::{blkptr::BlkPtr, dsl, pool::Pool, source::FileSource};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = FileSource::open(
        &a[1],
        a[2].parse().unwrap(),
        a.get(3).map(|x| x.parse().unwrap()),
    )
    .unwrap();
    let p = Pool::open(src).unwrap();
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    for d in dsl::list_datasets(&p, &mos).unwrap() {
        println!(
            "{} dir={} ds={} txg={} bp lsize={} fill={}",
            d.name, d.dir_obj, d.dataset_obj, d.creation_txg, d.bp.lsize, d.bp.fill
        );
    }
}
