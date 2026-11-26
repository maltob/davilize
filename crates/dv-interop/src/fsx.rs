//! File access for the converters.
//!
//! Natively this is `std::fs`. In the browser (`wasm32`) there is no file system, so files
//! live in an in-memory store: the page puts the files an import reads into it, and takes
//! the files an export wrote out of it ([`memfs`]).

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

/// An entry of a directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    path: PathBuf,
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn file_name(&self) -> OsString {
        self.path
            .file_name()
            .map(OsString::from)
            .unwrap_or_default()
    }
}

/// The entries of a directory, as `std::fs::read_dir` yields them.
pub type Entries = std::vec::IntoIter<io::Result<DirEntry>>;

/// A store of files by path, with directories implied by the paths. The browser build uses
/// it as its file system; it is plain Rust, so it is tested natively too.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub mod memfs {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    use super::{DirEntry, Entries};

    #[derive(Default)]
    struct Store {
        files: BTreeMap<String, Vec<u8>>,
        dirs: BTreeSet<String>,
    }

    thread_local! {
        static STORE: RefCell<Store> = RefCell::new(Store::default());
    }

    /// A path as a store key: forward slashes, no leading `./` or `/`, no trailing `/`.
    pub fn key(path: &Path) -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        let mut parts: Vec<&str> = Vec::new();
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        parts.join("/")
    }

    fn not_found(path: &Path) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no such file", path.display()),
        )
    }

    /// Adds or replaces a file.
    pub fn put(path: &Path, bytes: Vec<u8>) {
        STORE.with(|s| {
            s.borrow_mut().files.insert(key(path), bytes);
        });
    }

    /// A file's bytes.
    pub fn take(path: &Path) -> Option<Vec<u8>> {
        STORE.with(|s| s.borrow().files.get(&key(path)).cloned())
    }

    /// Every file path, sorted.
    pub fn list() -> Vec<String> {
        STORE.with(|s| s.borrow().files.keys().cloned().collect())
    }

    /// Removes every file and folder.
    pub fn clear() {
        STORE.with(|s| *s.borrow_mut() = Store::default());
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        take(path).ok_or_else(|| not_found(path))
    }

    pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        put(path.as_ref(), bytes.as_ref().to_vec());
        Ok(())
    }

    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        let k = key(path.as_ref());
        STORE.with(|s| {
            let mut s = s.borrow_mut();
            let mut current = String::new();
            for part in k.split('/').filter(|p| !p.is_empty()) {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(part);
                s.dirs.insert(current.clone());
            }
        });
        Ok(())
    }

    pub fn is_file(path: impl AsRef<Path>) -> bool {
        STORE.with(|s| s.borrow().files.contains_key(&key(path.as_ref())))
    }

    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        let k = key(path.as_ref());
        if k.is_empty() {
            return true;
        }
        let prefix = format!("{k}/");
        STORE.with(|s| {
            let s = s.borrow();
            s.dirs.contains(&k) || s.files.keys().any(|f| f.starts_with(&prefix))
        })
    }

    pub fn exists(path: impl AsRef<Path>) -> bool {
        is_file(path.as_ref()) || is_dir(path.as_ref())
    }

    pub fn read_dir(dir: impl AsRef<Path>) -> io::Result<Entries> {
        let dir = dir.as_ref();
        if !is_dir(dir) {
            return Err(not_found(dir));
        }
        let k = key(dir);
        let prefix = if k.is_empty() {
            String::new()
        } else {
            format!("{k}/")
        };
        let mut names = BTreeSet::new();
        STORE.with(|s| {
            let s = s.borrow();
            for path in s.files.keys().chain(s.dirs.iter()) {
                if let Some(rest) = path.strip_prefix(&prefix)
                    && let Some(first) = rest.split('/').next()
                    && !first.is_empty()
                {
                    names.insert(first.to_string());
                }
            }
        });
        let entries: Vec<io::Result<DirEntry>> = names
            .into_iter()
            .map(|name| {
                Ok(DirEntry {
                    path: dir.join(name),
                })
            })
            .collect();
        Ok(entries.into_iter())
    }

    /// An open file: reads and seeks over a copy of the bytes; a file made with
    /// [`File::create`] is stored when it is dropped.
    pub struct File {
        cursor: Cursor<Vec<u8>>,
        target: Option<PathBuf>,
    }

    impl File {
        pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
            Ok(File {
                cursor: Cursor::new(read(path)?),
                target: None,
            })
        }

        pub fn create(path: impl AsRef<Path>) -> io::Result<File> {
            let path = path.as_ref();
            put(path, Vec::new());
            Ok(File {
                cursor: Cursor::new(Vec::new()),
                target: Some(path.to_path_buf()),
            })
        }

        fn commit(&self) {
            if let Some(path) = &self.target {
                put(path, self.cursor.get_ref().clone());
            }
        }
    }

    impl Read for File {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.cursor.read(buf)
        }
    }

    impl Write for File {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.cursor.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.commit();
            Ok(())
        }
    }

    impl Seek for File {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.cursor.seek(pos)
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            self.commit();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{DirEntry, Entries};
    use std::io;
    use std::path::{Path, PathBuf};

    pub use std::fs::{File, create_dir_all, read, read_to_string, write};

    pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    pub fn open(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn create(path: &Path) -> io::Result<File> {
        File::create(path)
    }

    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    pub fn is_file(path: &Path) -> bool {
        path.is_file()
    }

    pub fn is_dir(path: &Path) -> bool {
        path.is_dir()
    }

    pub fn read_dir(dir: &Path) -> io::Result<Entries> {
        let entries: Vec<io::Result<DirEntry>> = std::fs::read_dir(dir)?
            .map(|e| e.map(|e| DirEntry { path: e.path() }))
            .collect();
        Ok(entries.into_iter())
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::memfs;
    use std::io;
    use std::path::{Path, PathBuf};

    pub use super::memfs::{File, create_dir_all, exists, is_dir, is_file, read, read_dir, write};

    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        String::from_utf8(read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// There are no links or relative roots to resolve in the store.
    pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    pub fn open(path: &Path) -> io::Result<File> {
        memfs::File::open(path)
    }

    pub fn create(path: &Path) -> io::Result<File> {
        memfs::File::create(path)
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::memfs;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::Path;

    #[test]
    fn keys_normalize_separators_and_dots() {
        assert_eq!(memfs::key(Path::new("./a\\b//c/../d.csv")), "a/b/d.csv");
        assert_eq!(memfs::key(Path::new("")), "");
    }

    #[test]
    fn files_and_directories_are_implied_by_paths() {
        memfs::clear();
        memfs::write(Path::new("proj/Report/definition/report.json"), b"{}").unwrap();
        memfs::write(Path::new("proj/Model/definition/tables/a.tmdl"), b"t").unwrap();
        assert!(memfs::is_dir(Path::new("proj")));
        assert!(memfs::is_dir(Path::new("proj/Report")));
        assert!(!memfs::is_dir(Path::new(
            "proj/Report/definition/report.json"
        )));
        assert!(memfs::is_file(Path::new(
            "proj/Report/definition/report.json"
        )));
        let names: Vec<String> = memfs::read_dir(Path::new("proj"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Model", "Report"]);
        assert!(memfs::read_dir(Path::new("missing")).is_err());
        memfs::clear();
        assert!(memfs::list().is_empty());
    }

    #[test]
    fn a_created_file_is_stored_when_dropped_and_can_be_reopened() {
        memfs::clear();
        {
            let mut file = memfs::File::create(Path::new("out/x.bin")).unwrap();
            file.write_all(b"hello world").unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(b"J").unwrap();
        }
        let mut text = String::new();
        memfs::File::open(Path::new("out/x.bin"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "Jello world");
        memfs::clear();
    }
}
