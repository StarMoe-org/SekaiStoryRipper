//! Content-keyed cache of deobfuscated bundles: `<root>/bundles/<bundleName>.<crc:08x>`.
//!
//! The key is `(bundleName, crc)`, so a bundle that changes on the CDN gets a new file and the
//! old one stays valid for older manifests. Writes go to a temp file in the same directory and
//! are renamed into place, so an interrupted download never leaves a truncated cache entry.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::manifest::BundleEntry;
use ripper_format::path as portable;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("bundle name cannot be stored portably: {0}")]
    Name(String),
    #[error("cache I/O at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

pub struct BundleCache {
    root: PathBuf,
}

impl BundleCache {
    pub fn new(cache_root: impl Into<PathBuf>) -> Self {
        Self {
            root: cache_root.into().join("bundles"),
        }
    }

    pub fn path(&self, entry: &BundleEntry) -> Result<PathBuf, CacheError> {
        portable::check_relative(&entry.bundle_name).map_err(CacheError::Name)?;
        let mut path = self.root.clone();
        path.extend(entry.bundle_name.split('/'));
        let leaf = format!(
            "{}.{:08x}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default(),
            entry.crc
        );
        path.set_file_name(leaf);
        Ok(path)
    }

    /// The cached UnityFS bytes, if present and the size matches the manifest.
    pub fn get(&self, entry: &BundleEntry) -> Result<Option<Vec<u8>>, CacheError> {
        let path = self.path(entry)?;
        match fs::read(&path) {
            Ok(data) if data.len() as u64 == entry.file_size => Ok(Some(data)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(CacheError::Io { path, source }),
        }
    }

    pub fn contains(&self, entry: &BundleEntry) -> Result<bool, CacheError> {
        let path = self.path(entry)?;
        Ok(fs::metadata(&path).is_ok_and(|m| m.len() == entry.file_size))
    }

    pub fn put(&self, entry: &BundleEntry, unityfs: &[u8]) -> Result<PathBuf, CacheError> {
        let path = self.path(entry)?;
        let io = |path: &PathBuf| {
            let path = path.clone();
            move |source| CacheError::Io { path, source }
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(&parent.to_path_buf()))?;
        }
        let mut temp = path.clone().into_os_string();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        fs::write(&temp, unityfs).map_err(io(&temp))?;
        fs::rename(&temp, &path).map_err(io(&path))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, crc: u32, file_size: u64) -> BundleEntry {
        BundleEntry {
            bundle_name: name.into(),
            category: None,
            file_size,
            dependencies: vec![],
            download_path: "ios1".into(),
            crc,
        }
    }

    #[test]
    fn stores_by_name_and_crc() {
        let dir = tempfile::tempdir().unwrap();
        let cache = BundleCache::new(dir.path());
        let v1 = entry("live2d/model/01ichika_normal", 0xdead_beef, 3);
        let v2 = entry("live2d/model/01ichika_normal", 1, 3);
        assert_eq!(cache.get(&v1).unwrap(), None);
        let path = cache.put(&v1, b"abc").unwrap();
        assert!(path.ends_with("bundles/live2d/model/01ichika_normal.deadbeef"));
        assert_eq!(cache.get(&v1).unwrap().as_deref(), Some(&b"abc"[..]));
        assert!(!cache.contains(&v2).unwrap());
    }

    #[test]
    fn a_size_mismatch_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let cache = BundleCache::new(dir.path());
        cache.put(&entry("a/b", 1, 3), b"abc").unwrap();
        assert_eq!(cache.get(&entry("a/b", 1, 4)).unwrap(), None);
    }

    #[test]
    fn rejects_unportable_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            BundleCache::new(dir.path())
                .path(&entry("a/../b", 1, 1))
                .is_err()
        );
    }
}
