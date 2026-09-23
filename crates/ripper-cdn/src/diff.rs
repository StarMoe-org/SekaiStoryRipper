//! Manifest diff by bundle name: added, removed, and changed (crc or size differs).

use serde::Serialize;

use crate::manifest::Manifest;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<Changed>,
    /// Same content, moved to another `ios{k}` directory (does not need a re-download).
    pub moved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub name: String,
    pub old_crc: u32,
    pub new_crc: u32,
    pub old_size: u64,
    pub new_size: u64,
}

impl ManifestDiff {
    pub fn between(old: &Manifest, new: &Manifest) -> Self {
        let mut diff = Self::default();
        for (name, entry) in &new.bundles {
            match old.bundles.get(name) {
                None => diff.added.push(name.clone()),
                Some(before) if before.crc != entry.crc || before.file_size != entry.file_size => {
                    diff.changed.push(Changed {
                        name: name.clone(),
                        old_crc: before.crc,
                        new_crc: entry.crc,
                        old_size: before.file_size,
                        new_size: entry.file_size,
                    });
                }
                Some(before) if before.download_path != entry.download_path => {
                    diff.moved.push(name.clone())
                }
                Some(_) => {}
            }
        }
        diff.removed = old
            .bundles
            .keys()
            .filter(|name| !new.bundles.contains_key(*name))
            .cloned()
            .collect();
        diff
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.moved.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::BundleEntry;

    fn manifest(entries: &[(&str, u32, u64, &str)]) -> Manifest {
        Manifest {
            bundles: entries
                .iter()
                .map(|&(name, crc, file_size, download_path)| {
                    (
                        name.to_owned(),
                        BundleEntry {
                            bundle_name: name.to_owned(),
                            category: None,
                            file_size,
                            dependencies: vec![],
                            download_path: download_path.to_owned(),
                            crc,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn classifies_every_kind_of_change() {
        let old = manifest(&[
            ("same", 1, 10, "ios1"),
            ("gone", 2, 10, "ios1"),
            ("edited", 3, 10, "ios1"),
            ("moved", 4, 10, "ios1"),
        ]);
        let new = manifest(&[
            ("same", 1, 10, "ios1"),
            ("new", 5, 10, "ios10"),
            ("edited", 6, 11, "ios10"),
            ("moved", 4, 10, "ios10"),
        ]);
        let diff = ManifestDiff::between(&old, &new);
        assert_eq!(diff.added, ["new"]);
        assert_eq!(diff.removed, ["gone"]);
        assert_eq!(diff.changed.len(), 1);
        assert_eq!((diff.changed[0].old_crc, diff.changed[0].new_crc), (3, 6));
        assert_eq!(diff.moved, ["moved"]);
        assert!(ManifestDiff::between(&new, &new).is_empty());
    }
}
