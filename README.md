# exhume_zfs

Read-only parsing of OpenZFS pools for the [Exhume](https://github.com/forensicxlab) toolkit.

The crate opens a single-disk pool from a raw image (or any `Read + Seek`
stream such as an Exhume `BodySlice`), selects the most recent valid
uberblock, and exposes every filesystem dataset through a small API that
`exhume_filesystem` maps onto its common `Filesystem` traits.

## What is read

| Layer | Support |
| --- | --- |
| Vdev labels | all four labels, XDR nvlist config, uberblock with the highest `txg` (slot size follows `ashift`) |
| Block pointers | DVA selection across the three copies, holes, embedded data |
| Checksums | fletcher2, fletcher4, sha256 (verified on every block read); other algorithms are read unverified |
| Compression | off, lzjb, zle, lz4, gzip 1-9, zstd (pure Rust: `flate2`, `ruzstd`) |
| DMU | dnodes, indirect blocks of any depth, object sets |
| MOS | object directory, DSL directories and datasets (child ZAPs, head datasets) |
| ZAP | microzap and fatzap (leaf scan, no hash lookup needed) |
| ZPL | master node, system attributes (registry and layouts), directories, regular files, symlinks, timestamps, uid/gid/mode |

Every dataset is exposed as a directory under a virtual root; a dataset's root
directory also lists its child datasets, like a mount with the default
mountpoints shows them. Identifiers pack the dataset slot (1-based position in
the sorted dataset table) in the upper 16 bits and the object number in the
lower 48, so a file keeps the same identifier across runs.

## Not supported (yet)

- Mirror, raidz and draid pools: a DVA on a vdev other than 0 is an error.
- Gang blocks.
- Encrypted datasets.
- Legacy znodes (datasets created before system attributes, ZPL version < 5).
- Snapshots and bookmarks are not listed; the pool is read at its last txg.
- Data checksum algorithms other than fletcher2/4 and sha256 (sha512, skein,
  edonr, blake3) are not verified.

## Usage

```rust
use exhume_zfs::fs::ZfsFS;

let fs = ZfsFS::new(std::fs::File::open("pool.img")?)?;
println!("{}", fs.metadata_pretty());
let node = fs.resolve_path("/rpool/ROOT/debian/etc/hostname")?;
let data = fs.read_all(&node)?;
```

The lower layers (`label`, `blkptr`, `pool`, `zap`, `dsl`, `zpl`) are public
for tools that need the raw structures.

## Tests

```sh
cargo test                                   # unit and robustness tests
EXHUME_ZFS_IMAGE=/path/to/partition.img \
EXHUME_ZFS_DATASET=rpool/ROOT/debian cargo test --release   # real pool
```

The image tests expect the pool used during development (a Debian 13 root
pool, `ashift=12`, lz4, 8 909 objects); point `EXHUME_ZFS_IMAGE` at the raw
partition, not at the whole disk.

The reader was checked against the mounted view of that pool (every regular
file hashed, `find -xdev` counts identical) and of eight pools built with
OpenZFS 2.4.4: lz4, zstd with 1 MiB records, gzip-9 with sha256 and
`ashift=9`, no compression with sha512, `dnodesize=auto` with blake3, lzjb
with a zle child dataset, a pool with two snapshots, and a two-way mirror
read from either member. Listing, sha256, symlink targets, mode, owner,
link count and mtime matched for every entry.

## License

GPL-3.0-only, like the rest of Exhume.
