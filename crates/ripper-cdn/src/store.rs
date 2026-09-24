//! Archive of decrypted manifests, one file per `(app version, asset version)`:
//! `<root>/manifests/<app>/<platform><version>.msgpack.zst` (CN `ios10`, JP `ios6.8.0.50`), plus a
//! `.meta.json` sidecar when the version needs more than its name to be downloaded (the JP asset
//! hash). Old versions are kept so updates can be diffed.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError};

/// What the archive keeps besides the manifest itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestMeta {
    /// JP: the asset hash of this asset version (bundles live under `{version}/{hash}/{platform}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_hash: Option<String>,
}

/// Orders asset versions by their dot-separated numeric parts (`9` < `10`, `6.8.0.9` < `6.8.0.50`).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a).cmp(&parts(b)).then_with(|| a.cmp(b))
}

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

    pub fn path(&self, app_version: &str, asset_version: &str) -> PathBuf {
        self.dir(app_version)
            .join(format!("{}{asset_version}.msgpack.zst", self.platform))
    }

    fn meta_path(&self, app_version: &str, asset_version: &str) -> PathBuf {
        self.dir(app_version)
            .join(format!("{}{asset_version}.meta.json", self.platform))
    }

    /// Stores the decrypted msgpack plaintext (zstd level 19; ~23 MB ciphertext compresses well)
    /// and, when it says anything, the sidecar.
    pub fn save(
        &self,
        app_version: &str,
        asset_version: &str,
        plain: &[u8],
        meta: &ManifestMeta,
    ) -> Result<PathBuf, StoreError> {
        let path = self.path(app_version, asset_version);
        let dir = self.dir(app_version);
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        if *meta != ManifestMeta::default() {
            let meta_path = self.meta_path(app_version, asset_version);
            let json = serde_json::to_vec_pretty(meta).expect("meta serializes");
            fs::write(&meta_path, json).map_err(io_error(&meta_path))?;
        }
        let compressed = zstd::encode_all(plain, 19).map_err(io_error(&path))?;
        let temp = path.with_extension("zst.tmp");
        fs::write(&temp, compressed).map_err(io_error(&temp))?;
        fs::rename(&temp, &path).map_err(io_error(&path))?;
        Ok(path)
    }

    pub fn meta(&self, app_version: &str, asset_version: &str) -> Result<ManifestMeta, StoreError> {
        let path = self.meta_path(app_version, asset_version);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| StoreError::Io {
                path,
                source: io::Error::new(io::ErrorKind::InvalidData, e),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ManifestMeta::default()),
            Err(error) => Err(io_error(&path)(error)),
        }
    }

    /// Loads a manifest; with a recorded asset hash, entries get the JP `downloadPath`.
    pub fn load(&self, app_version: &str, asset_version: &str) -> Result<Manifest, StoreError> {
        let path = self.path(app_version, asset_version);
        let compressed = fs::read(&path).map_err(io_error(&path))?;
        let plain = zstd::decode_all(compressed.as_slice()).map_err(io_error(&path))?;
        let mut manifest = Manifest::from_msgpack(&plain)?;
        if let Some(hash) = self.meta(app_version, asset_version)?.asset_hash {
            manifest.fill_download_path(&format!("{asset_version}/{hash}/{}", self.platform));
        }
        Ok(manifest)
    }

    /// Archived asset versions for an app version, ascending.
    pub fn versions(&self, app_version: &str) -> Result<Vec<String>, StoreError> {
        let dir = self.dir(app_version);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&dir)(error)),
        };
        let mut versions: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let version = name
                    .strip_prefix(&self.platform)?
                    .strip_suffix(".msgpack.zst")?;
                (!version.is_empty()).then(|| version.to_owned())
            })
            .collect();
        versions.sort_by(|a, b| compare_versions(a, b));
        Ok(versions)
    }

    pub fn latest(&self, app_version: &str) -> Result<Option<(String, Manifest)>, StoreError> {
        match self.versions(app_version)?.pop() {
            Some(version) => {
                let manifest = self.load(app_version, &version)?;
                Ok(Some((version, manifest)))
            }
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
        let none = ManifestMeta::default();
        store.save("6.4.0", "9", &sample(1), &none).unwrap();
        store.save("6.4.0", "10", &sample(2), &none).unwrap();
        store.save("6.0.0", "42", &sample(3), &none).unwrap();
        assert_eq!(store.versions("6.4.0").unwrap(), ["9", "10"]);
        let (version, manifest) = store.latest("6.4.0").unwrap().unwrap();
        assert_eq!((version.as_str(), manifest.bundles["a/b"].crc), ("10", 2));
        assert_eq!(manifest.bundles["a/b"].download_path, "ios1");
    }

    #[test]
    fn jp_versions_sort_numerically_and_get_their_download_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = ManifestStore::new(dir.path(), "ios");
        let meta = |hash: &str| ManifestMeta {
            asset_hash: Some(hash.into()),
        };
        store
            .save("6.8.1", "6.8.0.50", &jp_sample(), &meta("h50"))
            .unwrap();
        store
            .save("6.8.1", "6.8.0.9", &jp_sample(), &meta("h9"))
            .unwrap();
        assert_eq!(store.versions("6.8.1").unwrap(), ["6.8.0.9", "6.8.0.50"]);
        let (version, manifest) = store.latest("6.8.1").unwrap().unwrap();
        assert_eq!(version, "6.8.0.50");
        assert_eq!(manifest.bundles["a/b"].download_path, "6.8.0.50/h50/ios");
    }

    /// A JP manifest entry: no `downloadPath`, extra fields the tool ignores.
    fn jp_sample() -> Vec<u8> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            bundle_name: &'static str,
            cache_file_name: &'static str,
            hash: &'static str,
            category: &'static str,
            crc: u32,
            file_size: u64,
            dependencies: Vec<&'static str>,
            paths: Vec<&'static str>,
            is_builtin: bool,
        }
        #[derive(serde::Serialize)]
        struct Doc {
            version: &'static str,
            os: &'static str,
            bundles: std::collections::BTreeMap<&'static str, Entry>,
        }
        let doc = Doc {
            version: "6.8.0.50",
            os: "ios",
            bundles: [(
                "a/b",
                Entry {
                    bundle_name: "a/b",
                    cache_file_name: "30155e6c",
                    hash: "1d15f6a4",
                    category: "StartApp",
                    crc: 7,
                    file_size: 1,
                    dependencies: vec![],
                    paths: vec!["StartApp/a/b"],
                    is_builtin: false,
                },
            )]
            .into(),
        };
        rmp_serde::to_vec_named(&doc).unwrap()
    }
}
