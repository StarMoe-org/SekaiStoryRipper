//! `ripper unpack`: fetch bundles if needed and unpack them into `<out>/library/<bundleName>/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use ripper_cdn::{BundleCache, BundleEntry};
use ripper_convert::motion::BindingNames;
use ripper_convert::unpack::{UnpackOptions, is_up_to_date, moc3_ids, unpack_bundle};
use ripper_unity::{BundleSource, UnityRsBundle};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::fetch_cmd::{ensure_cached, load_manifest, select};

pub struct Args {
    pub names: Vec<String>,
    pub prefixes: Vec<String>,
    pub asset_version: Option<u32>,
    pub no_deps: bool,
    pub force: bool,
    pub keep_astc: bool,
}

/// Every Live2D parameter/part id seen in any unpacked moc3: `library/_index/live2d-ids.json`.
/// Binding path hashes depend only on the id, so the union over all models resolves any clip (D12).
#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownIds {
    parameters: BTreeSet<String>,
    parts: BTreeSet<String>,
}

impl KnownIds {
    fn path(library: &Path) -> PathBuf {
        library.join("_index").join("live2d-ids.json")
    }

    fn load(library: &Path) -> Result<Self> {
        match fs::read(Self::path(library)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("reading live2d-ids.json"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, library: &Path) -> Result<()> {
        let path = Self::path(library);
        fs::create_dir_all(path.parent().expect("has parent"))?;
        fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    fn names(&self) -> BindingNames {
        let mut names = BindingNames::default();
        let parameters: Vec<String> = self.parameters.iter().cloned().collect();
        let parts: Vec<String> = self.parts.iter().cloned().collect();
        names.add_model(&parameters, &parts);
        names
    }
}

fn bundle_dir(library: &Path, name: &str) -> PathBuf {
    let mut dir = library.to_path_buf();
    dir.extend(name.split('/'));
    dir
}

fn open(cache: &BundleCache, entry: &BundleEntry, unity_version: &str) -> Result<UnityRsBundle> {
    let bytes = cache
        .get(entry)?
        .with_context(|| format!("{} is not cached", entry.bundle_name))?;
    Ok(UnityRsBundle::open(
        &entry.bundle_name,
        bytes,
        unity_version,
    )?)
}

enum Outcome {
    UpToDate,
    Unpacked { files: usize, skipped: Vec<String> },
    Failed(String),
}

pub async fn run(config: &Config, args: Args) -> Result<()> {
    if args.names.is_empty() && args.prefixes.is_empty() {
        bail!("name at least one bundle or --prefix");
    }
    let (version, manifest) = load_manifest(config, args.asset_version)?;
    let entries = select(&manifest, &args.names, &args.prefixes, !args.no_deps)?;
    eprintln!(
        "{}{version}: {} bundles",
        config.cdn.platform,
        entries.len()
    );
    ensure_cached(config, entries.clone(), true).await?;

    let library = config.paths.out.join("library");
    let cache = BundleCache::new(&config.paths.cache);
    let unity = config.unity.version.clone();

    // Models first, so clips in this batch can name every parameter of the models alongside them.
    let mut known = KnownIds::load(&library)?;
    for entry in entries
        .iter()
        .filter(|e| e.bundle_name.starts_with("live2d/model/"))
    {
        for ids in moc3_ids(&open(&cache, entry, &unity)?)? {
            known.parameters.extend(ids.parameters);
            known.parts.extend(ids.parts);
        }
    }
    known.save(&library)?;
    let names = known.names();

    let ffmpeg = ripper_convert::movie::find_ffmpeg(&config.tools.ffmpeg);
    if ffmpeg.is_none()
        && entries
            .iter()
            .any(|e| e.bundle_name.starts_with("scenario/movie/"))
    {
        eprintln!(
            "warning: ffmpeg ({}) not found; movie ADX audio is kept without WAV",
            config.tools.ffmpeg.display()
        );
    }
    let options = UnpackOptions {
        ffmpeg,
        keep_astc: args.keep_astc,
    };
    let force = args.force;
    let outcomes: Vec<(String, Outcome)> = tokio::task::spawn_blocking(move || {
        entries
            .par_iter()
            .map(|entry| {
                let dir = bundle_dir(&library, &entry.bundle_name);
                if !force && is_up_to_date(&dir, entry.crc, &names) {
                    return (entry.bundle_name.clone(), Outcome::UpToDate);
                }
                let result = open(&cache, entry, &unity).and_then(|bundle| {
                    Ok(unpack_bundle(
                        &bundle,
                        &entry.bundle_name,
                        entry.crc,
                        &names,
                        &dir,
                        &options,
                    )?)
                });
                let outcome = match result {
                    Ok(record) => Outcome::Unpacked {
                        files: record.files.len(),
                        skipped: record.skipped,
                    },
                    Err(error) => Outcome::Failed(format!("{error:#}")),
                };
                (entry.bundle_name.clone(), outcome)
            })
            .collect()
    })
    .await?;

    let (mut unpacked, mut current, mut failed) = (0, 0, 0);
    for (name, outcome) in &outcomes {
        match outcome {
            Outcome::UpToDate => current += 1,
            Outcome::Unpacked { files, skipped } => {
                unpacked += 1;
                eprintln!(
                    "{name}: {files} files{}",
                    if skipped.is_empty() {
                        String::new()
                    } else {
                        format!(", {} skipped", skipped.len())
                    }
                );
                for reason in skipped.iter().take(5) {
                    eprintln!("    skipped {reason}");
                }
            }
            Outcome::Failed(reason) => {
                failed += 1;
                eprintln!("{name}: FAILED: {reason}");
            }
        }
    }
    println!(
        "{unpacked} unpacked, {current} up to date, {failed} failed -> {}",
        config.paths.out.join("library").display()
    );
    if failed > 0 {
        bail!("{failed} bundle(s) failed to unpack");
    }
    Ok(())
}
