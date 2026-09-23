//! Archive of decrypted manifests, one file per `(app version, asset version)`:
//! `<root>/manifests/<app>/<platform><N>.msgpack.zst`. Old versions are kept so updates can be diffed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("manifest store I/O at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_owned(),
        source,
    }
}

pub struct ManifestStore {
    root: PathBuf,
    platform: String,
}

impl ManifestStore {
    pub fn new(cache_root: impl Into<PathBuf>, platform: impl Into<String>) -> Self {
        Self {
            root: cache_root.into().join("manifests"),
            platform: platform.into(),
        }
    }

    fn dir(&self, app_version: &str) -> PathBuf {
        self.root.join(app_version)
    }

    pub fn path(&self, app_version: &str, asset_version: u32) -> PathBuf {
        self.dir(app_version)
            .join(format!("{}{asset_version}.msgpack.zst", self.platform))
    }

    /// Stores the decrypted msgpack plaintext (zstd level 19; ~23 MB ciphertext compresses well).
    pub fn save(
        &self,
        app_version: &str,
        asset_version: u32,
        plain: &[u8],
    ) -> Result<PathBuf, StoreError> {
        let path = self.path(app_version, asset_version);
        let dir = self.dir(app_version);
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        let compressed = zstd::encode_all(plain, 19).map_err(io_error(&path))?;
        let temp = path.with_extension("zst.tmp");
        fs::write(&temp, compressed).map_err(io_error(&temp))?;
        fs::rename(&temp, &path).map_err(io_error(&path))?;
        Ok(path)
    }

    pub fn load(&self, app_version: &str, asset_version: u32) -> Result<Manifest, StoreError> {
        let path = self.path(app_version, asset_version);
        let compressed = fs::read(&path).map_err(io_error(&path))?;
        let plain = zstd::decode_all(compressed.as_slice()).map_err(io_error(&path))?;
        Ok(Manifest::from_msgpack(&plain)?)
    }

    /// Archived asset versions for an app version, ascending.
    pub fn versions(&self, app_version: &str) -> Result<Vec<u32>, StoreError> {
        let dir = self.dir(app_version);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&dir)(error)),
        };
        let mut versions: Vec<u32> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                name.strip_prefix(&self.platform)?
                    .strip_suffix(".msgpack.zst")?
                    .parse()
                    .ok()
            })
            .collect();
        versions.sort_unstable();
        Ok(versions)
    }

    pub fn latest(&self, app_version: &str) -> Result<Option<(u32, Manifest)>, StoreError> {
        match self.versions(app_version)?.last() {
            Some(&version) => Ok(Some((version, self.load(app_version, version)?))),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(crc: u32) -> Vec<u8> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            bundle_name: &'static str,
            file_size: u64,
            download_path: &'static str,
            crc: u32,
        }
        #[derive(serde::Serialize)]
        struct Doc {
            bundles: std::collections::BTreeMap<&'static str, Entry>,
        }
        let doc = Doc {
            bundles: [(
                "a/b",
                Entry {
                    bundle_name: "a/b",
                    file_size: 1,
                    download_path: "ios1",
                    crc,
                },
            )]
            .into(),
        };
        rmp_serde::to_vec_named(&doc).unwrap()
    }

    #[test]
    fn saves_lists_and_loads_the_latest_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = ManifestStore::new(dir.path(), "ios");
        assert!(store.latest("6.4.0").unwrap().is_none());
        store.save("6.4.0", 9, &sample(1)).unwrap();
        store.save("6.4.0", 10, &sample(2)).unwrap();
        store.save("6.0.0", 42, &sample(3)).unwrap();
        assert_eq!(store.versions("6.4.0").unwrap(), [9, 10]);
        let (version, manifest) = store.latest("6.4.0").unwrap().unwrap();
        assert_eq!((version, manifest.bundles["a/b"].crc), (10, 2));
    }
}
