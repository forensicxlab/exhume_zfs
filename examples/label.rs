use exhume_zfs::{label, source::FileSource};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = label::read_best(&FileSource::open(&a[1], 0, None).unwrap()).unwrap();
    println!("{:?}", l.config.keys().collect::<Vec<_>>());
}
