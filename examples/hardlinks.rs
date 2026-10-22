use exhume_zfs::fs::ZfsFS;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fs = ZfsFS::new(std::fs::File::open(&a[1]).unwrap()).unwrap();
    let mut stack = vec![(fs.resolve_path(&a[2]).unwrap(), a[2].clone())];
    while let Some((n, p)) = stack.pop() {
        if n.attrs.is_dir() {
            for e in fs.list_dir(&n).unwrap() {
                if !e.is_dataset {
                    stack.push((fs.get_node(e.target_id).unwrap(), format!("{p}/{}", e.name)))
                }
            }
        } else if n.attrs.is_reg() && n.attrs.links > 1 {
            println!("links={} object={} {}", n.attrs.links, n.object, p)
        }
    }
}
