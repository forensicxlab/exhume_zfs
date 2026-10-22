// Emit, for one dataset, the same facts the `find` oracle records on a mounted view:
//   L <type> <size> <path>              (find -printf '%y %s %P')
//   H <sha256> <path>                   (sha256sum)
//   S <path> -> <target>                (find -type l -printf '%P -> %l')
//   T <path> <mode> <uid> <gid> <nlink> <mtime_sec>
// Child datasets are listed (as a mount point would be) but not entered.
use exhume_zfs::fs::ZfsFS;
use sha2::{Digest, Sha256};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fs = ZfsFS::new(std::fs::File::open(&a[1]).expect("open")).expect("pool");
    let root = fs.resolve_path(&a[2]).expect("dataset path");
    let mut stack = vec![(root, String::new())];
    while let Some((n, p)) = stack.pop() {
        let at = &n.attrs;
        if !p.is_empty() {
            let ty = match at.mode & 0o170000 {
                0o040000 => 'd',
                0o100000 => 'f',
                0o120000 => 'l',
                0o010000 => 'p',
                0o020000 => 'c',
                0o060000 => 'b',
                0o140000 => 's',
                _ => '?',
            };
            println!("L {ty} {} {p}", at.size);
            println!(
                "T {p} {:o} {} {} {} {}",
                at.mode & 0o7777,
                at.uid,
                at.gid,
                at.links,
                at.mtime.0
            );
            if at.is_symlink() {
                println!("S {p} -> {}", at.symlink.clone().unwrap_or_default());
            }
            if at.is_reg() {
                match fs.read_all(&n) {
                    Ok(d) => println!("H {:x} {p}", Sha256::digest(&d)),
                    Err(e) => println!("E read {p}: {e}"),
                }
            }
        }
        if at.is_dir() {
            match fs.list_dir(&n) {
                Ok(es) => {
                    for e in es {
                        let cp = if p.is_empty() {
                            e.name.clone()
                        } else {
                            format!("{p}/{}", e.name)
                        };
                        match fs.get_node(e.target_id) {
                            Ok(c) => {
                                if e.is_dataset {
                                    // mount point: stat it, do not descend
                                    let ca = &c.attrs;
                                    println!("L d {} {cp}", ca.size);
                                    println!(
                                        "T {cp} {:o} {} {} {} {}",
                                        ca.mode & 0o7777,
                                        ca.uid,
                                        ca.gid,
                                        ca.links,
                                        ca.mtime.0
                                    );
                                } else {
                                    stack.push((c, cp));
                                }
                            }
                            Err(err) => println!("E node {cp}: {err}"),
                        }
                    }
                }
                Err(e) => println!("E list {p}: {e}"),
            }
        }
    }
}
