//! High-level read-only view of a pool: every dataset is a directory under the pool root.
//!
//! The virtual root (slot 0) lists the top-level datasets; a dataset's root directory also lists
//! its child datasets, like a mount with default mountpoints. Identifiers pack the dataset slot
//! (position in the sorted dataset table, 1-based) into the upper 16 bits and the object number
//! into the lower 48.

use crate::blkptr::BlkPtr;
use crate::dsl::{self, Dataset};
use crate::nvlist::Value;
use crate::pool::{Dnode, Pool};
use crate::source::ReadSeekSource;
use crate::zpl::{Attrs, Zpl};
use serde_json::{Value as Json, json};
use std::io::{self, Read, Seek};

const OBJECT_BITS: u32 = 48;
const OBJECT_MASK: u64 = (1 << OBJECT_BITS) - 1;
pub const MAX_SLOTS: usize = 1 << 16;
/// Identifier of the virtual root (slot 0). Non-zero because front-ends treat 0 as "no record".
pub const ROOT_ID: u64 = 1;

pub fn pack_id(slot: usize, object: u64) -> Option<u64> {
    (slot < MAX_SLOTS && object <= OBJECT_MASK).then_some(((slot as u64) << OBJECT_BITS) | object)
}

pub fn unpack_id(id: u64) -> (usize, u64) {
    ((id >> OBJECT_BITS) as usize, id & OBJECT_MASK)
}

fn bad<T>(m: impl Into<String>) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, m.into()))
}

#[derive(Debug, Clone)]
pub struct Node {
    pub slot: usize,
    pub object: u64,
    pub attrs: Attrs,
    /// None for the virtual root.
    pub dnode: Option<Dnode>,
}

impl Node {
    pub fn id(&self) -> u64 {
        pack_id(self.slot, self.object).unwrap_or(self.object)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub target_id: u64,
    /// DT_* kind from the directory (4 dir, 8 file, 10 symlink); 4 for datasets.
    pub kind: u8,
    pub is_dataset: bool,
}

pub struct ZfsFS<T: Read + Seek> {
    pub pool: Pool<ReadSeekSource<T>>,
    /// Sorted by name; slot = index + 1.
    pub datasets: Vec<Dataset>,
    zpls: Vec<Option<Zpl>>,
}

impl<T: Read + Seek> ZfsFS<T> {
    pub fn new(stream: T) -> io::Result<Self> {
        let pool = Pool::open(ReadSeekSource::new(stream)?)?;
        let mos = pool.objset_meta(&BlkPtr::parse(&pool.uberblock.rootbp))?;
        let datasets = dsl::list_datasets(&pool, &mos)?;
        if datasets.len() >= MAX_SLOTS {
            return bad("too many datasets for a packed identifier");
        }
        let zpls = datasets
            .iter()
            .map(|d| match Zpl::open(&pool, d) {
                Ok(z) => Some(z),
                Err(e) => {
                    log::warn!("zfs: dataset {} is not a readable filesystem: {e}", d.name);
                    None
                }
            })
            .collect();
        Ok(ZfsFS {
            pool,
            datasets,
            zpls,
        })
    }

    fn zpl(&self, slot: usize) -> io::Result<&Zpl> {
        match self.zpls.get(slot.wrapping_sub(1)) {
            Some(Some(z)) => Ok(z),
            Some(None) => bad(format!(
                "dataset {} is not a filesystem",
                self.datasets[slot - 1].name
            )),
            None => bad(format!("unknown dataset slot {slot}")),
        }
    }

    /// Identifier of the virtual root directory.
    pub fn root_id(&self) -> u64 {
        ROOT_ID
    }

    pub fn block_size(&self) -> u64 {
        match self.pool.config.get("vdev_tree") {
            Some(Value::List(t)) => match t.get("ashift") {
                Some(Value::U64(a)) if *a < 32 => 1 << a,
                _ => 4096,
            },
            _ => 4096,
        }
    }

    pub fn get_node(&self, id: u64) -> io::Result<Node> {
        let (slot, object) = unpack_id(id);
        if slot == 0 {
            if object != ROOT_ID {
                return bad("the virtual root has a single object");
            }
            let ts = self.pool.uberblock.timestamp;
            return Ok(Node {
                slot: 0,
                object: ROOT_ID,
                attrs: Attrs {
                    mode: 0o040555,
                    links: 2,
                    atime: (ts, 0),
                    mtime: (ts, 0),
                    ctime: (ts, 0),
                    crtime: (ts, 0),
                    ..Attrs::default()
                },
                dnode: None,
            });
        }
        let z = self.zpl(slot)?;
        let dnode = z.dnode(&self.pool, object)?;
        if dnode.is_free() {
            return bad(format!("object {object} is free"));
        }
        let attrs = z.attrs(&dnode)?;
        Ok(Node {
            slot,
            object,
            attrs,
            dnode: Some(dnode),
        })
    }

    pub fn root_node(&self) -> io::Result<Node> {
        self.get_node(ROOT_ID)
    }

    fn dataset_entry(&self, slot: usize, name: String) -> Option<Entry> {
        let z = self.zpls.get(slot - 1)?.as_ref()?;
        Some(Entry {
            name,
            target_id: pack_id(slot, z.root)?,
            kind: 4,
            is_dataset: true,
        })
    }

    pub fn list_dir(&self, node: &Node) -> io::Result<Vec<Entry>> {
        if node.slot == 0 {
            return Ok(self
                .datasets
                .iter()
                .enumerate()
                .filter(|(_, d)| !d.name.contains('/'))
                .filter_map(|(i, d)| self.dataset_entry(i + 1, d.name.clone()))
                .collect());
        }
        let Some(dn) = &node.dnode else {
            return bad("not a directory");
        };
        if !node.attrs.is_dir() {
            return bad("not a directory");
        }
        let z = self.zpl(node.slot)?;
        let mut out: Vec<Entry> = z
            .readdir(&self.pool, dn)?
            .into_iter()
            .filter_map(|e| {
                Some(Entry {
                    target_id: pack_id(node.slot, e.object)?,
                    name: e.name,
                    kind: e.kind,
                    is_dataset: false,
                })
            })
            .collect();
        if node.object == z.root {
            let prefix = format!("{}/", self.datasets[node.slot - 1].name);
            for (i, d) in self.datasets.iter().enumerate() {
                let Some(child) = d.name.strip_prefix(&prefix) else {
                    continue;
                };
                if child.contains('/') {
                    continue;
                }
                // A mounted dataset shadows the (empty) mount point directory the
                // parent keeps for it, like the kernel shows it.
                if let Some(pos) = out.iter().position(|e| e.name == child) {
                    log::debug!(
                        "zfs: dataset {} shadows the directory of the same name",
                        d.name
                    );
                    out.remove(pos);
                }
                out.extend(self.dataset_entry(i + 1, child.to_string()));
            }
        }
        Ok(out)
    }

    pub fn resolve_path(&self, path: &str) -> io::Result<Node> {
        let mut cur = self.root_node()?;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let e = self
                .list_dir(&cur)?
                .into_iter()
                .find(|e| e.name == part)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("path component not found: {part:?}"),
                    )
                })?;
            cur = self.get_node(e.target_id)?;
        }
        Ok(cur)
    }

    /// Read `length` bytes starting at `offset` (clamped to the file size).
    pub fn read_slice(&self, node: &Node, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if node.attrs.is_symlink() {
            return Ok(node
                .attrs
                .symlink
                .clone()
                .unwrap_or_default()
                .into_bytes()
                .into_iter()
                .skip(offset as usize)
                .take(length)
                .collect());
        }
        let Some(dn) = &node.dnode else {
            return bad("not a file");
        };
        if !node.attrs.is_reg() {
            return bad("not a regular file");
        }
        let size = node.attrs.size;
        if offset >= size || length == 0 {
            return Ok(Vec::new());
        }
        let end = size.min(offset.saturating_add(length as u64));
        let bs = dn.datablksz;
        if bs == 0 {
            return bad("data block size is zero");
        }
        let mut out = Vec::with_capacity((end - offset) as usize);
        let mut blk = offset / bs;
        while (blk * bs) < end {
            let data = self.pool.read_block(dn, blk)?;
            let start = offset.saturating_sub(blk * bs) as usize;
            let stop = ((end - blk * bs).min(bs) as usize).min(data.len());
            if start < stop {
                out.extend_from_slice(&data[start..stop]);
            }
            blk += 1;
        }
        Ok(out)
    }

    pub fn read_all(&self, node: &Node) -> io::Result<Vec<u8>> {
        let size =
            usize::try_from(node.attrs.size).map_err(|_| io::Error::other("file too large"))?;
        self.read_slice(node, 0, size)
    }

    /// Number of objects in all filesystem datasets (the `fill` of each objset root).
    pub fn object_count(&self) -> u64 {
        self.datasets
            .iter()
            .zip(&self.zpls)
            .filter(|(_, z)| z.is_some())
            .map(|(d, _)| d.bp.fill)
            .sum()
    }

    pub fn metadata_json(&self) -> Json {
        let c = &self.pool.config;
        let s = |k: &str| match c.get(k) {
            Some(Value::Str(v)) => Json::String(v.clone()),
            _ => Json::Null,
        };
        let n = |k: &str| match c.get(k) {
            Some(Value::U64(v)) => json!(v),
            _ => Json::Null,
        };
        let u = &self.pool.uberblock;
        json!({
            "pool_name": s("name"),
            "pool_guid": n("pool_guid"),
            "hostname": s("hostname"),
            "label_version": n("version"),
            "uberblock": {"txg": u.txg, "version": u.version, "timestamp": u.timestamp, "label": u.label, "slot": u.slot},
            "ashift": self.block_size().trailing_zeros(),
            "datasets": self.datasets.iter().enumerate().map(|(i, d)| json!({
                "slot": i + 1,
                "name": d.name,
                "creation_txg": d.creation_txg,
                "objects": d.bp.fill,
                "filesystem": self.zpls[i].is_some(),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn metadata_pretty(&self) -> String {
        let m = self.metadata_json();
        let mut out = format!(
            "ZFS pool {} (guid {}), txg {}, ashift {}\n",
            m["pool_name"].as_str().unwrap_or("?"),
            m["pool_guid"],
            m["uberblock"]["txg"],
            m["ashift"]
        );
        for d in m["datasets"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  [{}] {} (txg {}, {} objects{})\n",
                d["slot"],
                d["name"].as_str().unwrap_or("?"),
                d["creation_txg"],
                d["objects"],
                if d["filesystem"] == true {
                    ""
                } else {
                    ", not a filesystem"
                }
            ));
        }
        out
    }
}

/// True when the stream starts with a valid ZFS label and uberblock.
pub fn has_zfs_label<T: Read + Seek>(stream: T) -> bool {
    match ReadSeekSource::new(stream) {
        Ok(s) => crate::label::read_best(&s).is_some(),
        Err(_) => false,
    }
}
