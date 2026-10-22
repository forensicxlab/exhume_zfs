use exhume_zfs::{blkptr::BlkPtr, dsl, pool::Pool, source::FileSource, zpl::Zpl};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(&a[3]);
    let p = Pool::open(FileSource::open(&a[1], 0, None).unwrap()).unwrap();
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    let ds = dsl::list_datasets(&p, &mos)
        .unwrap()
        .into_iter()
        .find(|d| d.name == a[2])
        .unwrap();
    let z = Zpl::open(&p, &ds).unwrap();
    let mut stack = vec![(z.root, out.clone())];
    while let Some((obj, path)) = stack.pop() {
        let dn = z.dnode(&p, obj).unwrap();
        let at = z.attrs(&dn).unwrap();
        if at.is_dir() {
            fs::create_dir_all(&path).unwrap();
            for e in z.readdir(&p, &dn).unwrap() {
                stack.push((e.object, path.join(&e.name)))
            }
            let _ = fs::set_permissions(
                &path,
                fs::Permissions::from_mode(0o700 | (at.mode as u32 & 0o777)),
            );
        } else if at.is_reg() {
            fs::write(&path, z.read_file(&p, &dn, at.size).unwrap()).unwrap();
            let _ = fs::set_permissions(
                &path,
                fs::Permissions::from_mode(0o600 | (at.mode as u32 & 0o777)),
            );
        } else if at.is_symlink() {
            let _ = symlink(at.symlink.unwrap_or_default(), &path);
        }
    }
}
