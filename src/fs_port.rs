//! Thin filesystem interface so hooks can be tested against `FakeFileSystem` instead of disk.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The file operations the hooks need, and nothing else.
pub trait FileSystem {
    /// Size in bytes of a regular file, `None` for missing files and directories.
    fn file_size(&self, path: &Path) -> Option<u64>;
    /// Whole regular file as UTF-8 text (BOM stripped, invalid bytes replaced).
    fn read_text(&self, path: &Path) -> Option<String>;
    /// Up to `len` bytes starting at `start`.
    fn read_range(&self, path: &Path, start: u64, len: usize) -> Option<Vec<u8>>;
    /// Creates parent folders and writes `text`; `false` when that fails.
    fn write_text(&self, path: &Path, text: &str) -> bool;
}

/// The real disk.
pub struct DiskFileSystem;

impl FileSystem for DiskFileSystem {
    fn file_size(&self, path: &Path) -> Option<u64> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file().then_some(meta.len())
    }

    fn read_text(&self, path: &Path) -> Option<String> {
        self.file_size(path)?;
        let bytes = std::fs::read(path).ok()?;
        Some(decode_text(&bytes))
    }

    fn read_range(&self, path: &Path, start: u64, len: usize) -> Option<Vec<u8>> {
        let mut file = std::fs::File::open(path).ok()?;
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut buffer = Vec::with_capacity(len);
        file.take(len as u64).read_to_end(&mut buffer).ok()?;
        Some(buffer)
    }

    fn write_text(&self, path: &Path, text: &str) -> bool {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, text).is_ok()
    }
}

/// Decodes file bytes the way .NET `ReadAllText(path, UTF8)` does: BOM dropped, bad bytes replaced.
///
/// Example: `decode_text(b"\xEF\xBB\xBFhi")` -> `"hi"`
pub fn decode_text(bytes: &[u8]) -> String {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(body).into_owned()
}

#[cfg(test)]
pub mod fakes {
    use super::FileSystem;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// In-memory filesystem keyed by exact path.
    #[derive(Default)]
    pub struct FakeFileSystem {
        pub files: RefCell<HashMap<PathBuf, Vec<u8>>>,
    }

    impl FakeFileSystem {
        pub fn with_file(self, path: &str, body: &[u8]) -> Self {
            self.files
                .borrow_mut()
                .insert(PathBuf::from(path), body.to_vec());
            self
        }

        pub fn text_of(&self, path: &str) -> Option<String> {
            let files = self.files.borrow();
            files
                .get(Path::new(path))
                .map(|b| String::from_utf8_lossy(b).into_owned())
        }
    }

    impl FileSystem for FakeFileSystem {
        fn file_size(&self, path: &Path) -> Option<u64> {
            self.files.borrow().get(path).map(|b| b.len() as u64)
        }

        fn read_text(&self, path: &Path) -> Option<String> {
            let files = self.files.borrow();
            files.get(path).map(|b| super::decode_text(b))
        }

        fn read_range(&self, path: &Path, start: u64, len: usize) -> Option<Vec<u8>> {
            let files = self.files.borrow();
            let body = files.get(path)?;
            let from = (start as usize).min(body.len());
            let to = (from + len).min(body.len());
            Some(body[from..to].to_vec())
        }

        fn write_text(&self, path: &Path, text: &str) -> bool {
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), text.as_bytes().to_vec());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file(name: &str, body: &[u8]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("cc-hooks-test-{}-{name}", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn decode_text_strips_bom_and_replaces_bad_bytes() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi"), "hi");
        assert_eq!(decode_text(b"a\xFFb"), "a\u{FFFD}b");
    }

    #[test]
    fn disk_file_size_ignores_directories() {
        assert_eq!(DiskFileSystem.file_size(&std::env::temp_dir()), None);
        let path = scratch_file("size", b"12345");
        assert_eq!(DiskFileSystem.file_size(&path), Some(5));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn disk_read_text_and_range() {
        let path = scratch_file("range", b"\xEF\xBB\xBFhello world");
        assert_eq!(
            DiskFileSystem.read_text(&path).as_deref(),
            Some("hello world")
        );
        assert_eq!(
            DiskFileSystem.read_range(&path, 9, 5),
            Some(b"world".to_vec())
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn disk_write_text_creates_parents() {
        let dir = std::env::temp_dir().join(format!("cc-hooks-test-dir-{}", std::process::id()));
        let path = dir.join("nested").join("f.txt");
        assert!(DiskFileSystem.write_text(&path, "x"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
        let _ = std::fs::remove_dir_all(dir);
    }
}
