//! Tests on a real pool. Set EXHUME_ZFS_IMAGE to a file holding a single-disk pool (a raw
//! partition, e.g. extracted with dd) and EXHUME_ZFS_DATASET to the dataset to walk.
//! Without the variable the tests return immediately.
use exhume_zfs::{blkptr::BlkPtr, dsl, pool::Pool, source::FileSource, zpl::Zpl};
use sha2::{Digest, Sha256};

fn image() -> Option<(String, String)> {
    Some((
        std::env::var("EXHUME_ZFS_IMAGE").ok()?,
        std::env::var("EXHUME_ZFS_DATASET").unwrap_or("rpool/ROOT/debian".into()),
    ))
}

#[test]
fn pool_datasets_and_uberblock() {
    let Some((img, _)) = image() else { return };
    let p = Pool::open(FileSource::open(&img, 0, None).unwrap()).unwrap();
    assert_eq!(p.uberblock.txg, 61);
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    let names: Vec<_> = dsl::list_datasets(&p, &mos)
        .unwrap()
        .into_iter()
        .map(|d| (d.name, d.creation_txg))
        .collect();
    assert_eq!(
        names,
        [
            ("rpool".into(), 1),
            ("rpool/ROOT".into(), 6),
            ("rpool/ROOT/debian".into(), 8)
        ]
    );
}

#[test]
fn walk_every_file_and_hash_known_ones() {
    let Some((img, dsname)) = image() else { return };
    let p = Pool::open(FileSource::open(&img, 0, None).unwrap()).unwrap();
    let mos = p.objset_meta(&BlkPtr::parse(&p.uberblock.rootbp)).unwrap();
    let ds = dsl::list_datasets(&p, &mos)
        .unwrap()
        .into_iter()
        .find(|d| d.name == dsname)
        .unwrap();
    let z = Zpl::open(&p, &ds).unwrap();
    let (mut files, mut dirs) = (0, 0);
    let mut stack = vec![(z.root, String::new())];
    while let Some((obj, path)) = stack.pop() {
        let dn = z.dnode(&p, obj).unwrap();
        let at = z.attrs(&dn).unwrap();
        if at.is_dir() {
            dirs += 1;
            stack.extend(
                z.readdir(&p, &dn)
                    .unwrap()
                    .into_iter()
                    .map(|e| (e.object, format!("{path}/{}", e.name))),
            );
        } else if at.is_reg() {
            files += 1;
            let data = z.read_file(&p, &dn, at.size).unwrap();
            assert_eq!(data.len() as u64, at.size, "{path}");
            if path == "/usr/lib/systemd/systemd-networkd" {
                // has a hole in the middle: the digest was checked against the dpkg md5sums
                assert_eq!(
                    hex(&Sha256::digest(&data)),
                    "e0dadaea4df940b087492ca56905b51265b590572b784242889f445d13f4bd59"
                );
            }
        }
    }
    assert_eq!((dirs, files), (1037, 7145));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn high_level_view() {
    let Some((img, _)) = image() else { return };
    let fs = exhume_zfs::fs::ZfsFS::new(std::fs::File::open(&img).unwrap()).unwrap();
    let root = fs.root_node().unwrap();
    assert_eq!((root.id(), fs.root_id()), (exhume_zfs::fs::ROOT_ID, 1));
    assert!(fs.get_node(0).is_err(), "0 is never a valid identifier");
    let top: Vec<_> = fs
        .list_dir(&root)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(top, ["rpool"]);
    let deb = fs.resolve_path("/rpool/ROOT/debian").unwrap();
    assert!(deb.attrs.is_dir());
    assert_eq!(fs.resolve_path("rpool/ROOT").unwrap().slot, 2);
    let names: Vec<_> = fs
        .list_dir(&deb)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&"etc".to_string()) && names.contains(&"usr".to_string()));
    // a child dataset shows up under its parent
    let ids: Vec<_> = fs
        .list_dir(&fs.resolve_path("/rpool").unwrap())
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.is_dataset))
        .collect();
    assert!(ids.contains(&("ROOT".to_string(), true)));
    // whole file and slices agree
    let link = fs
        .resolve_path("/rpool/ROOT/debian/etc/os-release")
        .unwrap();
    assert!(link.attrs.is_symlink());
    assert_eq!(fs.read_all(&link).unwrap(), b"../usr/lib/os-release");
    let f = fs
        .resolve_path("/rpool/ROOT/debian/usr/lib/os-release")
        .unwrap();
    let all = fs.read_all(&f).unwrap();
    assert_eq!(
        hex(&Sha256::digest(&all)),
        "e249e69c32d81350fdd6b05ac5575255f14e380ba14b8454acea14e49f088bfa"
    );
    assert_eq!(fs.read_slice(&f, 5, 10).unwrap(), all[5..15]);
    assert_eq!(
        fs.read_slice(&f, all.len() as u64 - 3, 100).unwrap(),
        all[all.len() - 3..]
    );
    assert!(fs.read_slice(&f, all.len() as u64, 10).unwrap().is_empty());
    // slice across the hole of a sparse file
    let big = fs
        .resolve_path("/rpool/ROOT/debian/usr/lib/systemd/systemd-networkd")
        .unwrap();
    let whole = fs.read_all(&big).unwrap();
    assert_eq!(
        fs.read_slice(&big, 0x17_f000, 0x2_2000).unwrap(),
        whole[0x17_f000..0x1a_1000]
    );
    assert!(
        whole[0x18_0000..0x1a_0000].iter().all(|b| *b == 0),
        "hole reads as zeros"
    );
    assert!(fs.resolve_path("/rpool/nope").is_err());
}
