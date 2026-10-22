//! Dataset hierarchy: object directory -> root_dataset -> dsl_dir -> children / head dataset.
use crate::blkptr::BlkPtr;
use crate::pool::{Dnode, Pool};
use crate::source::Source;
use crate::zap::{self, ZapEntry};
use std::io;

fn u64at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

#[derive(Debug, Clone)]
pub struct Dataset {
    pub name: String,
    pub dir_obj: u64,
    pub dataset_obj: u64,
    pub creation_txg: u64,
    pub bp: BlkPtr,
}

pub fn zap_of<S: Source>(p: &Pool<S>, dn: &Dnode) -> io::Result<Vec<ZapEntry>> {
    Ok(zap::parse(&p.read_object(dn)?, dn.datablksz as usize))
}

/// Walk the DSL directory tree from the object directory and list every dataset (not snapshots).
pub fn list_datasets<S: Source>(p: &Pool<S>, mos: &Dnode) -> io::Result<Vec<Dataset>> {
    let root = p.dnode_at(mos, 1)?;
    let od = zap_of(p, &root)?;
    let root_dir = od
        .iter()
        .find(|e| e.name == "root_dataset")
        .and_then(|e| e.as_u64())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no root_dataset"))?;
    let pool_name = match p.config.get("name") {
        Some(crate::nvlist::Value::Str(s)) => s.clone(),
        _ => "pool".into(),
    };
    let mut out = Vec::new();
    let mut stack = vec![(root_dir, pool_name)];
    while let Some((dir_obj, name)) = stack.pop() {
        let dir = p.dnode_at(mos, dir_obj)?;
        let head = u64at(&dir.bonus, 8);
        let child_zap = u64at(&dir.bonus, 32);
        let ds = p.dnode_at(mos, head)?;
        out.push(Dataset {
            name: name.clone(),
            dir_obj,
            dataset_obj: head,
            creation_txg: u64at(&ds.bonus, 56),
            bp: BlkPtr::parse(&ds.bonus[128..256]),
        });
        if child_zap != 0 {
            let cz = p.dnode_at(mos, child_zap)?;
            for e in zap_of(p, &cz)? {
                if e.name.starts_with('$') {
                    continue; // $ORIGIN, $FREE, $MOS
                }
                if let Some(o) = e.as_u64() {
                    stack.push((o, format!("{name}/{}", e.name)));
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
