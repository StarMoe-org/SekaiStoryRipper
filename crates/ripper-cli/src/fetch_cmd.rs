//! `ripper fetch`: download bundles (by name or prefix, plus their dependencies) into the cache.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ripper_cdn::download::{FetchStatus, Verifier, fetch_all};
use ripper_cdn::{BundleCache, BundleEntry, CdnClient, Manifest, ManifestStore};

use crate::config::Config;

pub struct Args {
    pub names: Vec<String>,
    pub prefixes: Vec<String>,
    pub asset_version: Option<u32>,
    pub no_deps: bool,
    pub no_verify: bool,
}

/// Selected bundles plus their transitive `dependencies`, in a stable order.
pub fn select(
    manifest: &Manifest,
    names: &[String],
    prefixes: &[String],
    with_deps: bool,
) -> Result<Vec<BundleEntry>> {
    let mut queue: VecDeque<String> = VecDeque::new();
    for name in names {
        if !manifest.bundles.contains_key(name) {
            bail!("{name}: not in the manifest");
        }
        queue.push_back(name.clone());
    }
    for prefix in prefixes {
        let before = queue.len();
        queue.extend(
            manifest
                .bundles
                .keys()
                .filter(|n| n.starts_with(prefix.as_str()))
                .cloned(),
        );
        if queue.len() == before {
            bail!("prefix {prefix:?} matches no bundle");
        }
    }
    let mut selected = BTreeSet::new();
    while let Some(name) = queue.pop_front() {
        let Some(entry) = manifest.bundles.get(&name) else {
            eprintln!("warning: dependency {name} is not in the manifest");
            continue;
        };
        if selected.insert(name) && with_deps {
            queue.extend(entry.dependencies.iter().cloned());
        }
    }
    Ok(selected
        .into_iter()
        .map(|name| manifest.bundles[&name].clone())
        .collect())
}

pub async fn run(config: &Config, args: Args) -> Result<()> {
    if args.names.is_empty() && args.prefixes.is_empty() {
        bail!("name at least one bundle or --prefix");
    }
    let app = &config.cdn.app_version;
    let store = ManifestStore::new(&config.paths.cache, &config.cdn.platform);
    let (version, manifest) = match args.asset_version {
        Some(version) => (version, store.load(app, version)?),
        None => store
            .latest(app)?
            .context("no archived manifest; run `ripper manifest` first")?,
    };
    let entries = select(&manifest, &args.names, &args.prefixes, !args.no_deps)?;
    let total: u64 = entries.iter().map(|e| e.file_size).sum();
    eprintln!(
        "{}{version}: {} bundles, {:.1} MB",
        config.cdn.platform,
        entries.len(),
        total as f64 / 1e6
    );

    let verify: Option<Verifier> = (!args.no_verify).then(|| {
        Arc::new(|entry: &BundleEntry, data: &[u8]| {
            let crc = ripper_unity::content_crc32(&entry.bundle_name, data.to_vec())
                .map_err(|e| e.to_string())?;
            if crc == entry.crc {
                Ok(())
            } else {
                Err(format!(
                    "content crc {crc:08x} != manifest {:08x}",
                    entry.crc
                ))
            }
        }) as Verifier
    });

    let client = Arc::new(CdnClient::new(config.cdn.clone(), None)?);
    let cache = Arc::new(BundleCache::new(&config.paths.cache));
    let count = entries.len();
    let mut done = 0usize;
    let outcomes = fetch_all(client, cache, entries, verify, |outcome| {
        done += 1;
        let status = match &outcome.status {
            FetchStatus::Cached => "cached".to_owned(),
            FetchStatus::Downloaded { bytes } => {
                format!("downloaded {:.1} MB", *bytes as f64 / 1e6)
            }
            FetchStatus::Failed(reason) => format!("FAILED: {reason}"),
        };
        eprintln!("[{done}/{count}] {}: {status}", outcome.name);
    })
    .await;

    let failed = outcomes
        .iter()
        .filter(|o| matches!(o.status, FetchStatus::Failed(_)))
        .count();
    let cached = outcomes
        .iter()
        .filter(|o| o.status == FetchStatus::Cached)
        .count();
    println!(
        "{} downloaded, {cached} already cached, {failed} failed",
        outcomes.len() - cached - failed
    );
    if failed > 0 {
        bail!("{failed} bundle(s) failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let entry = |name: &str, deps: &[&str]| {
            (
                name.to_owned(),
                BundleEntry {
                    bundle_name: name.into(),
                    category: None,
                    file_size: 1,
                    dependencies: deps.iter().map(|d| d.to_string()).collect(),
                    download_path: "ios1".into(),
                    crc: 0,
                },
            )
        };
        Manifest {
            bundles: [
                entry("scenario/effect/hologram", &["shader/particles"]),
                entry("scenario/effect/snow_03", &["shader/particles"]),
                entry("shader/particles", &[]),
                entry("font/common", &["custom_profile/font"]),
                entry("custom_profile/font", &[]),
            ]
            .into(),
        }
    }

    #[test]
    fn selects_names_prefixes_and_transitive_dependencies() {
        let names: Vec<String> = select(
            &manifest(),
            &["font/common".into()],
            &["scenario/effect/".into()],
            true,
        )
        .unwrap()
        .into_iter()
        .map(|e| e.bundle_name)
        .collect();
        assert_eq!(
            names,
            [
                "custom_profile/font",
                "font/common",
                "scenario/effect/hologram",
                "scenario/effect/snow_03",
                "shader/particles"
            ]
        );
        let without = select(&manifest(), &["font/common".into()], &[], false).unwrap();
        assert_eq!(without.len(), 1);
        assert!(select(&manifest(), &["nope".into()], &[], true).is_err());
    }
}
