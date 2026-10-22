// Count like `find -xdev`: one hit per path (hard links counted at every path).
use exhume_zfs::fs::ZfsFS;
use std::collections::HashSet;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fs = ZfsFS::new(std::fs::File::open(&a[1]).unwrap()).unwrap();
    let start = fs.resolve_path(&a[2]).unwrap();
    let slot = start.slot;
    let (mut paths_f, mut paths_d, mut paths_l, mut paths_o) = (0u64, 0u64, 0u64, 0u64);
    let mut objs_f = HashSet::new();
    let mut stack = vec![start];
    while let Some(n) = stack.pop() {
        let a = &n.attrs;
        if a.is_dir() {
            paths_d += 1;
            for e in fs.list_dir(&n).unwrap() {
                if e.is_dataset {
                    continue;
                } // -xdev: stay on this dataset
                let (s, _) = exhume_zfs::fs::unpack_id(e.target_id);
                if s != slot {
                    continue;
                }
                stack.push(fs.get_node(e.target_id).unwrap());
            }
        } else if a.is_reg() {
            paths_f += 1;
            objs_f.insert(n.object);
        } else if a.is_symlink() {
            paths_l += 1
        } else {
            paths_o += 1
        }
    }
    println!(
        "find -type f: {paths_f} paths ({} distinct objects); -type d: {paths_d}; -type l: {paths_l}; other: {paths_o}",
        objs_f.len()
    );
}
