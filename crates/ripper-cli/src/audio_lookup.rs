//! Cue lookup over unpacked ACBs: `library/<bundle>/**/<x>.cues.json` (`ripper-acb` v1).
//!
//! The resolver asks "which files play cue C of bundle B?"; answers are paths relative to the
//! library root, `/`-separated, so they can go straight into an episode index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ripper_format::audio::AcbIndex;

/// `(acb path, waveform paths)`, both relative to the library root.
pub type CueFiles = (String, Vec<String>);

pub struct AudioLookup {
    library: PathBuf,
    /// bundle name -> [(acb path relative to library, index)], loaded on first use.
    loaded: Mutex<HashMap<String, Vec<(String, AcbIndex)>>>,
}

fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    Some(
        relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn find_indexes(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_indexes(&path, out);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".cues.json"))
        {
            out.push(path);
        }
    }
}

impl AudioLookup {
    pub fn new(library: impl Into<PathBuf>) -> Self {
        Self {
            library: library.into(),
            loaded: Mutex::new(HashMap::new()),
        }
    }

    fn indexes_of(&self, bundle: &str) -> Vec<(String, AcbIndex)> {
        let mut loaded = self.loaded.lock().expect("lookup lock poisoned");
        if let Some(indexes) = loaded.get(bundle) {
            return indexes.clone();
        }
        let mut dir = self.library.clone();
        dir.extend(bundle.split('/'));
        let mut paths = Vec::new();
        find_indexes(&dir, &mut paths);
        paths.sort();
        let indexes: Vec<(String, AcbIndex)> = paths
            .into_iter()
            .filter_map(|path| {
                let index: AcbIndex = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
                let acb = path.with_file_name(&index.acb);
                Some((relative_to(&self.library, &acb)?, index))
            })
            .collect();
        loaded.insert(bundle.to_owned(), indexes.clone());
        indexes
    }

    /// Files for `cue` in `bundle`, or `None` when no ACB of the bundle has that cue.
    pub fn cue(&self, bundle: &str, cue: &str) -> Option<CueFiles> {
        for (acb, index) in self.indexes_of(bundle) {
            if let Some(files) = index.cue_files(cue) {
                let dir = acb
                    .rsplit_once('/')
                    .map_or(String::new(), |(d, _)| format!("{d}/"));
                return Some((
                    acb.clone(),
                    files.into_iter().map(|f| format!("{dir}{f}")).collect(),
                ));
            }
        }
        None
    }

    /// Every cue name of a bundle (to build an authoritative SE name index).
    pub fn cue_names(&self, bundle: &str) -> Vec<String> {
        self.indexes_of(bundle)
            .into_iter()
            .flat_map(|(_, index)| index.cues.into_iter().map(|c| c.name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ripper_format::audio::{Cue, Waveform};

    #[test]
    fn finds_cues_across_the_acbs_of_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_dir = dir.path().join("sound/scenario/se/se");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let index = AcbIndex {
            format: ripper_format::audio::FORMAT.into(),
            version: ripper_format::audio::VERSION,
            acb: "se_pack00001_b.acb".into(),
            cues: vec![Cue {
                name: "se00001_b".into(),
                cue_id: 0,
                waveforms: vec!["e2".into()],
            }],
            waveforms: [(
                "e2".to_owned(),
                Waveform {
                    file: "se_pack00001_b.audio/e2.wav".into(),
                    encoding: "hca".into(),
                    hca: None,
                },
            )]
            .into(),
            warnings: vec![],
        };
        std::fs::write(
            bundle_dir.join("se_pack00001_b.cues.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let lookup = AudioLookup::new(dir.path());
        assert_eq!(
            lookup.cue("sound/scenario/se/se", "se00001_b"),
            Some((
                "sound/scenario/se/se/se_pack00001_b.acb".to_owned(),
                vec!["sound/scenario/se/se/se_pack00001_b.audio/e2.wav".to_owned()]
            ))
        );
        assert_eq!(lookup.cue("sound/scenario/se/se", "nope"), None);
        assert_eq!(lookup.cue_names("sound/scenario/se/se"), ["se00001_b"]);
    }
}
