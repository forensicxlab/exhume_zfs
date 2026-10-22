//! ZPL (POSIX layer): master node, system attributes (SA), directories and file contents.
use crate::dsl::{Dataset, zap_of};
use crate::pool::{Dnode, Pool};
use crate::source::Source;
use std::collections::HashMap;
use std::io;

const SA_MAGIC: u32 = 0x002F_505A;
const BONUS_SA: u8 = 44;

fn bad<T>(m: &str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, m.to_string()))
}

#[derive(Debug, Clone, Default)]
pub struct Attrs {
    pub mode: u64,
    pub size: u64,
    pub uid: u64,
    pub gid: u64,
    pub links: u64,
    pub parent: u64,
    pub generation: u64,
    pub atime: (u64, u64),
    pub mtime: (u64, u64),
    pub ctime: (u64, u64),
    pub crtime: (u64, u64),
    pub symlink: Option<String>,
}

impl Attrs {
    pub fn is_dir(&self) -> bool {
        self.mode & 0o170000 == 0o040000
    }
    pub fn is_reg(&self) -> bool {
        self.mode & 0o170000 == 0o100000
    }
    pub fn is_symlink(&self) -> bool {
        self.mode & 0o170000 == 0o120000
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub object: u64,
    pub kind: u8, // DT_*
}

pub struct Zpl {
    pub meta: Dnode,
    pub root: u64,
    /// attribute name -> (number, fixed length; 0 = variable)
    registry: HashMap<String, (u16, u16)>,
    /// layout number -> ordered attribute numbers
    layouts: HashMap<u64, Vec<u16>>,
    by_num: HashMap<u16, String>,
}

impl Zpl {
    pub fn open<S: Source>(pool: &Pool<S>, ds: &Dataset) -> io::Result<Self> {
        let meta = pool.objset_meta(&ds.bp)?;
        let master = pool.dnode_at(&meta, 1)?;
        let m = zap_of(pool, &master)?;
        let find = |n: &str| m.iter().find(|e| e.name == n).and_then(|e| e.as_u64());
        let root = find("ROOT").map_or_else(|| bad("no ROOT in master node"), Ok)?;
        let sa_obj = find("SA_ATTRS");
        let (mut registry, mut layouts, mut by_num) =
            (HashMap::new(), HashMap::new(), HashMap::new());
        if let Some(o) = sa_obj {
            let sa = pool.dnode_at(&meta, o)?;
            // SA_ATTRS has two child ZAP objects: LAYOUTS and REGISTRY, named in its own ZAP.
            for e in zap_of(pool, &sa)? {
                let Some(obj) = e.as_u64() else { continue };
                let z = zap_of(pool, &pool.dnode_at(&meta, obj)?)?;
                match e.name.as_str() {
                    "REGISTRY" => {
                        for r in z {
                            if let Some(v) = r.as_u64() {
                                let (num, len) = ((v & 0xffff) as u16, ((v >> 24) & 0xffff) as u16);
                                by_num.insert(num, r.name.clone());
                                registry.insert(r.name, (num, len));
                            }
                        }
                    }
                    "LAYOUTS" => {
                        for l in z {
                            let ids = l
                                .data
                                .chunks_exact(2)
                                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                                .collect();
                            if let Ok(n) = l.name.parse::<u64>() {
                                layouts.insert(n, ids);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Zpl {
            meta,
            root,
            registry,
            layouts,
            by_num,
        })
    }

    pub fn dnode<S: Source>(&self, pool: &Pool<S>, obj: u64) -> io::Result<Dnode> {
        pool.dnode_at(&self.meta, obj)
    }

    pub fn attrs(&self, dn: &Dnode) -> io::Result<Attrs> {
        if dn.bonustype != BONUS_SA {
            return bad("object has no system attributes (legacy znode not supported)");
        }
        let b = &dn.bonus;
        if b.len() < 8 || u32::from_le_bytes(b[..4].try_into().unwrap()) != SA_MAGIC {
            return bad("bad SA magic");
        }
        let info = u16::from_le_bytes([b[4], b[5]]);
        let (layout, hdr) = ((info & 0x3ff) as u64, ((info >> 10) as usize) * 8);
        let Some(order) = self.layouts.get(&layout) else {
            return bad("unknown SA layout");
        };
        let (mut off, mut var) = (hdr, 0usize);
        let mut a = Attrs::default();
        for num in order {
            let name = self.by_num.get(num).map(String::as_str).unwrap_or("");
            let flen = self.registry.get(name).map_or(0, |r| r.1) as usize;
            let len = if flen != 0 {
                flen
            } else {
                let l = b
                    .get(6 + var * 2..8 + var * 2)
                    .map(|x| u16::from_le_bytes([x[0], x[1]]) as usize);
                var += 1;
                l.unwrap_or(0)
            };
            let Some(v) = b.get(off..off + len) else {
                break;
            };
            let u = |i: usize| u64::from_le_bytes(v[i..i + 8].try_into().unwrap());
            match name {
                "ZPL_MODE" => a.mode = u(0),
                "ZPL_SIZE" => a.size = u(0),
                "ZPL_UID" => a.uid = u(0),
                "ZPL_GID" => a.gid = u(0),
                "ZPL_LINKS" => a.links = u(0),
                "ZPL_PARENT" => a.parent = u(0),
                "ZPL_GEN" => a.generation = u(0),
                "ZPL_ATIME" => a.atime = (u(0), u(8)),
                "ZPL_MTIME" => a.mtime = (u(0), u(8)),
                "ZPL_CTIME" => a.ctime = (u(0), u(8)),
                "ZPL_CRTIME" => a.crtime = (u(0), u(8)),
                "ZPL_SYMLINK" => a.symlink = Some(String::from_utf8_lossy(v).into_owned()),
                _ => {}
            }
            off += len;
        }
        Ok(a)
    }

    pub fn readdir<S: Source>(&self, pool: &Pool<S>, dn: &Dnode) -> io::Result<Vec<DirEntry>> {
        Ok(zap_of(pool, dn)?
            .into_iter()
            .filter_map(|e| {
                let v = e.as_u64()?;
                Some(DirEntry {
                    name: e.name,
                    object: v & 0x0000_ffff_ffff_ffff,
                    kind: (v >> 60) as u8,
                })
            })
            .collect())
    }

    pub fn read_file<S: Source>(
        &self,
        pool: &Pool<S>,
        dn: &Dnode,
        size: u64,
    ) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        let mut blk = 0;
        while (out.len() as u64) < size {
            out.extend(pool.read_block(dn, blk)?);
            blk += 1;
        }
        out.truncate(size as usize);
        Ok(out)
    }
}

impl Attrs {
    /// `ls -l` style mode string, e.g. `drwxr-xr-x`.
    pub fn permissions(&self) -> String {
        let t = match self.mode & 0o170000 {
            0o040000 => 'd',
            0o120000 => 'l',
            0o100000 => '-',
            0o020000 => 'c',
            0o060000 => 'b',
            0o010000 => 'p',
            0o140000 => 's',
            _ => '?',
        };
        let mut out = String::from(t);
        for (i, c) in "rwxrwxrwx".chars().enumerate() {
            out.push(if self.mode & (0o400 >> i) != 0 {
                c
            } else {
                '-'
            });
        }
        out
    }
}
