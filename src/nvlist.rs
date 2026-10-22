//! XDR-encoded nvlist decoder (the format used in ZFS vdev labels).
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    U64(u64),
    I64(i64),
    U32(u32),
    Str(String),
    List(NvList),
    Array(Vec<NvList>),
    U64Array(Vec<u64>),
    Other(u32),
}

pub type NvList = BTreeMap<String, Value>;

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_be_bytes(self.b.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn u64(&mut self) -> Option<u64> {
        let v = u64::from_be_bytes(self.b.get(self.p..self.p + 8)?.try_into().ok()?);
        self.p += 8;
        Some(v)
    }
    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let s = self.b.get(self.p..self.p + n)?;
        self.p += (n + 3) & !3;
        Some(String::from_utf8_lossy(s).into_owned())
    }
}

/// Decode a top-level nvlist: 4-byte header (encoding, endian) followed by the XDR stream.
pub fn decode(buf: &[u8]) -> Option<NvList> {
    if buf.len() < 4 || buf[0] != 1 {
        return None; // only XDR encoding
    }
    let mut r = Rd { b: buf, p: 4 };
    list(&mut r, 0)
}

fn list(r: &mut Rd, depth: u32) -> Option<NvList> {
    if depth > 16 {
        return None;
    }
    let _version = r.u32()?;
    let _flag = r.u32()?;
    let mut out = NvList::new();
    loop {
        let start = r.p;
        let size = r.u32()? as usize;
        if size == 0 {
            r.u32()?; // the terminator is two zero words (encoded and decoded size)
            return Some(out);
        }
        let _ = r.u32()?; // decoded size
        let name = r.string()?;
        let ty = r.u32()?;
        let n = r.u32()?;
        let v = match ty {
            1 => Value::Bool(true),
            2..=6 => Value::U32(r.u32()?), // byte/int16/uint16/int32/uint32 are 4 wide in XDR
            7 => Value::I64(r.u64()? as i64),
            8 => Value::U64(r.u64()?),
            9 => Value::Str(r.string()?),
            16 => {
                let mut a = Vec::new();
                for _ in 0..n {
                    a.push(r.u64()?);
                }
                Value::U64Array(a)
            }
            21 => Value::Bool(r.u32()? != 0),
            19 => Value::List(list(r, depth + 1)?),
            20 => {
                let mut a = Vec::new();
                for _ in 0..n {
                    a.push(list(r, depth + 1)?);
                }
                Value::Array(a)
            }
            _ => Value::Other(ty),
        };
        if let Value::Other(_) = v {
            r.p = start + size; // skip unknown payload
        }
        let _ = n;
        out.insert(name, v);
    }
}
