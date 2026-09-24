//! `ripper manifest`: fetch (or import) the manifest, archive it, and diff against the previous one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ripper_cdn::{Manifest, ManifestDiff, ManifestMeta, ManifestStore, Region};

use crate::config::Config;

pub struct Args {
    pub asset_version: Option<String>,
    /// A manifest someone already decrypted (D4 compatibility): msgpack, or JSON `{"bundles": ...}`.
    pub from_file: Option<PathBuf>,
    pub refresh: bool,
    pub diff_out: Option<PathBuf>,
}

/// Converts an imported manifest file to the msgpack plaintext the store archives.
fn import(path: &Path) -> Result<(Vec<u8>, Option<String>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        // The oracle scripts record the CDN version they fetched as `_cdn_version`; a JP manifest
        // names its own (`version`).
        let version = ["_cdn_version", "version"]
            .iter()
            .find_map(|key| json.get(key)?.as_str().map(str::to_owned));
        let plain = rmp_serde::to_vec_named(&json)?;
        Manifest::from_msgpack(&plain).context("JSON is not a manifest")?;
        return Ok((plain, version));
    }
    Manifest::from_msgpack(&bytes).context("file is neither a manifest JSON nor msgpack")?;
    Ok((bytes, None))
}

pub async fn run(config: &Config, args: Args) -> Result<()> {
    let app = config.cdn.app_version.clone();
    let store = ManifestStore::new(&config.paths.cache, &config.cdn.platform);
    let previous = store.latest(&app)?;

    let (version, manifest) = if let Some(path) = &args.from_file {
        let (plain, recorded) = import(path)?;
        let Some(version) = args.asset_version.clone().or(recorded) else {
            bail!("pass --asset-version for an imported manifest (it does not record its version)");
        };
        let meta = ManifestMeta {
            asset_hash: config.cdn.jp.asset_hash.clone(),
        };
        if config.cdn.region == Region::Jp && meta.asset_hash.is_none() {
            bail!("set cdn.jp.asset_hash for an imported JP manifest (bundle URLs need it)");
        }
        let saved = store.save(&app, &version, &plain, &meta)?;
        println!(
            "imported {} as {}{version} -> {}",
            path.display(),
            config.cdn.platform,
            saved.display()
        );
        (version.clone(), store.load(&app, &version)?)
    } else {
        let mut cdn = config.cdn.clone();
        cdn.asset_version = args.asset_version.clone().or(cdn.asset_version);
        let client = config.cdn_client(cdn)?;
        let asset = client.asset_version().await?;
        let version = asset.version.clone();
        if !args.refresh && store.versions(&app)?.contains(&version) {
            println!(
                "{}{version}: already archived at {}",
                config.cdn.platform,
                store.path(&app, &version).display()
            );
            (version.clone(), store.load(&app, &version)?)
        } else {
            let plain = client.manifest_plain(&asset).await?;
            let meta = ManifestMeta {
                asset_hash: asset.hash.clone(),
            };
            let saved = store.save(&app, &version, &plain, &meta)?;
            println!(
                "{}{version}: fetched -> {}",
                config.cdn.platform,
                saved.display()
            );
            (version.clone(), store.load(&app, &version)?)
        }
    };
    println!(
        "{} bundles, {:.1} GB",
        manifest.bundles.len(),
        manifest.bundles.values().map(|b| b.file_size).sum::<u64>() as f64 / 1e9
    );

    match previous {
        Some((old_version, old)) if old_version != version => {
            let diff = ManifestDiff::between(&old, &manifest);
            println!(
                "since {}{old_version}: {} added, {} changed, {} removed, {} moved",
                config.cdn.platform,
                diff.added.len(),
                diff.changed.len(),
                diff.removed.len(),
                diff.moved.len()
            );
            if let Some(path) = &args.diff_out {
                std::fs::write(path, serde_json::to_vec_pretty(&diff)?)?;
                println!("diff -> {}", path.display());
            }
        }
        _ if args.diff_out.is_some() => println!("no earlier archived manifest to diff against"),
        _ => {}
    }
    Ok(())
}
