use exhume_zfs::{blkptr::BlkPtr, dsl, pool::Pool, source::FileSource, zpl::Zpl};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = FileSource::open(&a[1], 0, None).unwrap();
    let p = Pool::open(src).unwrap();
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    let ds = dsl::list_datasets(&p, &mos)
        .unwrap()
        .into_iter()
        .find(|d| d.name == a[2])
        .unwrap();
    let z = Zpl::open(&p, &ds).unwrap();
    let mut obj = z.root;
    for part in a[3].split('/').filter(|s| !s.is_empty()) {
        let dn = z.dnode(&p, obj).unwrap();
        obj = z
            .readdir(&p, &dn)
            .unwrap()
            .into_iter()
            .find(|e| e.name == part)
            .expect("not found")
            .object;
    }
    let dn = z.dnode(&p, obj).unwrap();
    let at = z.attrs(&dn).unwrap();
    println!("{:?}", at);
    if at.is_dir() {
        for e in z.readdir(&p, &dn).unwrap() {
            println!("{:>8} {} {}", e.object, e.kind, e.name);
        }
    } else if at.is_reg() {
        let d = z.read_file(&p, &dn, at.size).unwrap();
        println!(
            "{} bytes: {}",
            d.len(),
            String::from_utf8_lossy(&d[..d.len().min(200)])
        );
    }
}
