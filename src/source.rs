use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

/// Random-access byte source (a raw image or a partition window inside one).
pub trait Source {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct FileSource {
    file: File,
    base: u64,
    len: u64,
}

impl FileSource {
    pub fn open(path: &str, base: u64, len: Option<u64>) -> io::Result<Self> {
        let file = File::open(path)?;
        let total = file.metadata()?.len();
        let len = len.unwrap_or(total.saturating_sub(base));
        Ok(Self { file, base, len })
    }
}

impl Source for FileSource {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.read_exact_at(buf, self.base + offset)
    }
    fn len(&self) -> u64 {
        self.len
    }
}

/// Adapter for any `Read + Seek` stream (an Exhume `BodySlice`, a `File`, a `Cursor`).
pub struct ReadSeekSource<T: std::io::Read + std::io::Seek> {
    inner: std::cell::RefCell<T>,
    len: u64,
}

impl<T: std::io::Read + std::io::Seek> ReadSeekSource<T> {
    pub fn new(mut inner: T) -> io::Result<Self> {
        let len = inner.seek(std::io::SeekFrom::End(0))?;
        Ok(Self {
            inner: std::cell::RefCell::new(inner),
            len,
        })
    }
}

impl<T: std::io::Read + std::io::Seek> Source for ReadSeekSource<T> {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut s = self.inner.borrow_mut();
        s.seek(std::io::SeekFrom::Start(offset))?;
        s.read_exact(buf)
    }
    fn len(&self) -> u64 {
        self.len
    }
}
