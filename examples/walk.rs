use exhume_zfs::{blkptr::BlkPtr, dsl, pool::Pool, source::FileSource, zpl::Zpl};
use std::collections::HashSet;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = Pool::open(FileSource::open(&a[1], 0, None).unwrap()).unwrap();
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    let ds = dsl::list_datasets(&p, &mos)
        .unwrap()
        .into_iter()
        .find(|d| d.name == a[2])
        .unwrap();
    let z = Zpl::open(&p, &ds).unwrap();
    let (mut dirs, mut files, mut links, mut other, mut bytes, mut errs) = (0, 0, 0, 0, 0u64, 0);
    let mut seen = HashSet::new();
    let mut stack = vec![(z.root, String::new())];
    while let Some((obj, path)) = stack.pop() {
        let dn = match z.dnode(&p, obj) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{path}: dnode: {e}");
                errs += 1;
                continue;
            }
        };
        let at = match z.attrs(&dn) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{path}: attrs: {e}");
                errs += 1;
                continue;
            }
        };
        seen.insert(obj);
        if at.is_dir() {
            dirs += 1;
            match z.readdir(&p, &dn) {
                Ok(es) => {
                    for e in es {
                        if !seen.contains(&e.object) {
                            stack.push((e.object, format!("{path}/{}", e.name)))
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{path}: readdir: {e}");
                    errs += 1
                }
            }
        } else if at.is_reg() {
            files += 1;
            match z.read_file(&p, &dn, at.size) {
                Ok(d) => bytes += d.len() as u64,
                Err(e) => {
                    eprintln!("{path}: read: {e}");
                    errs += 1
                }
            }
        } else if at.is_symlink() {
            links += 1
        } else {
            other += 1
        }
    }
    println!(
        "dirs {dirs} files {files} symlinks {links} other {other} bytes {bytes} errors {errs} distinct objs {}",
        seen.len()
    );
}
