//! `ripper manifest`: fetch (or import) the manifest, archive it, and diff against the previous one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ripper_cdn::{CdnClient, Manifest, ManifestDiff, ManifestStore};

use crate::config::Config;

pub struct Args {
    pub asset_version: Option<u32>,
    /// A manifest someone already decrypted (D4 compatibility): msgpack, or JSON `{"bundles": ...}`.
    pub from_file: Option<PathBuf>,
    pub refresh: bool,
    pub diff_out: Option<PathBuf>,
}

/// Converts an imported manifest file to the msgpack plaintext the store archives.
fn import(path: &Path) -> Result<(Vec<u8>, Option<u32>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        // The oracle scripts record the CDN version they fetched as `_cdn_version`.
        let version = json
            .get("_cdn_version")
            .and_then(|v| v.as_str()?.parse().ok());
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
        let Some(version) = args.asset_version.or(recorded) else {
            bail!("pass --asset-version for an imported manifest (it does not record ios{{N}})");
        };
        let saved = store.save(&app, version, &plain)?;
        println!(
            "imported {} as {}{version} -> {}",
            path.display(),
            config.cdn.platform,
            saved.display()
        );
        (version, Manifest::from_msgpack(&plain)?)
    } else {
        let mut cdn = config.cdn.clone();
        cdn.asset_version = args.asset_version.or(cdn.asset_version);
        let client = CdnClient::new(cdn, config.manifest_key()?)?;
        let version = client.asset_version().await?;
        if !args.refresh && store.versions(&app)?.contains(&version) {
            println!(
                "{}{version}: already archived at {}",
                config.cdn.platform,
                store.path(&app, version).display()
            );
            (version, store.load(&app, version)?)
        } else {
            let plain = client.manifest_plain(version).await?;
            let saved = store.save(&app, version, &plain)?;
            println!(
                "{}{version}: fetched -> {}",
                config.cdn.platform,
                saved.display()
            );
            (version, Manifest::from_msgpack(&plain)?)
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
