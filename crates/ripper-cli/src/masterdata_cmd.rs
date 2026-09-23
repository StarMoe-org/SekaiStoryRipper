//! `ripper masterdata`: fetch the masterdata tables the resolver needs into `<cache>/masterdata/`.
//!
//! The source is the configurable `masterdata.url_template` (decision D15). Tables are cached as
//! fetched; `meta.json` records where and when, so a plan can say which masterdata it used.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ripper_cdn::CdnClient;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Tables used by the resolver (catalog, Live2D rules, AreaTalk costumes).
pub const TABLES: &[&str] = &[
    "unitStories",
    "eventStories",
    "cardEpisodes",
    "cards",
    "specialStories",
    "character2ds",
    "costume2ds",
];

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Meta {
    pub url_template: String,
    /// Unix seconds per table.
    pub fetched_at: std::collections::BTreeMap<String, u64>,
}

pub fn dir(config: &Config) -> PathBuf {
    config.paths.cache.join("masterdata")
}

/// Reads a cached table, fetching it first if it is missing.
pub async fn table(config: &Config, name: &str) -> Result<serde_json::Value> {
    let path = dir(config).join(format!("{name}.json"));
    if !path.exists() {
        fetch(config, &[name], false).await?;
    }
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("{} is not JSON", path.display()))
}

pub async fn fetch(config: &Config, names: &[&str], refresh: bool) -> Result<()> {
    let dir = dir(config);
    std::fs::create_dir_all(&dir)?;
    let meta_path = dir.join("meta.json");
    let mut meta: Meta = std::fs::read(&meta_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let client = CdnClient::new(config.cdn.clone(), None)?;
    for name in names {
        let path = dir.join(format!("{name}.json"));
        if !refresh && path.exists() {
            continue;
        }
        let url = config.masterdata.url_template.replace("{name}", name);
        let bytes = client
            .get_url(&url)
            .await
            .with_context(|| format!("fetching masterdata {name}"))?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .with_context(|| format!("{url} is not JSON"))?;
        write_atomic(&path, &bytes)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        meta.fetched_at.insert((*name).to_owned(), now);
        eprintln!("masterdata {name}: {} KB", bytes.len() / 1024);
    }
    meta.url_template = config.masterdata.url_template.clone();
    write_atomic(&meta_path, &serde_json::to_vec_pretty(&meta)?)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

pub async fn run(config: &Config, refresh: bool) -> Result<()> {
    fetch(config, TABLES, refresh).await?;
    println!("{} tables in {}", TABLES.len(), dir(config).display());
    Ok(())
}
