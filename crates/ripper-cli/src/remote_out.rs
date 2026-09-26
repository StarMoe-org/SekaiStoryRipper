//! `--out s3://bucket/prefix` (ADR-0012): commands write to a local staging directory as usual,
//! then [`RemoteOut::publish`] uploads what changed.
//!
//! - The staging directory is a cache (`<cache>/s3-out/<bucket>/<prefix>`); deleting it is safe.
//! - A ledger (`.ripper-s3.json` in the staging root) records every object known to be in the
//!   bucket: with the SHA-256 of what was uploaded, or without a hash for files only seen listed
//!   in a remote bundle record. Unchanged files are not uploaded again.
//! - When a bundle's record is missing locally, [`RemoteOut::hydrate_bundle`] fetches the record
//!   and the bundle's JSON files (what later steps read back: scenarios, cue indexes) and marks the
//!   rest as present remotely, so the bundle counts as unpacked without downloading its binaries.
//! - Uploads keep the local invariant "a bundle with a record is complete": files first, then
//!   `_ripper.json` records, then episode indexes, then `ripper.lock.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ripper_format::unpack::{RECORD_FILE, UnpackRecord};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::s3::{S3Client, S3Config, S3Location};

const LEDGER_FILE: &str = ".ripper-s3.json";

/// What was uploaded from (or downloaded to) a staged file. Size and modification time let an
/// unchanged file be skipped without reading it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Entry {
    sha256: String,
    size: u64,
    mtime_ns: u128,
}

/// Object path (relative to the prefix) → what was uploaded, or `None` for an object only known
/// from a remote record.
type Ledger = BTreeMap<String, Option<Entry>>;

pub struct RemoteOut {
    client: Arc<S3Client>,
    location: S3Location,
    staging: PathBuf,
    ledger: Mutex<Ledger>,
    concurrency: usize,
}

impl std::fmt::Debug for RemoteOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RemoteOut({} via {})",
            self.location,
            self.staging.display()
        )
    }
}

#[derive(Debug, Default)]
pub struct PublishSummary {
    pub uploaded: usize,
    pub unchanged: usize,
    pub bytes: u64,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn stat(path: &Path) -> Result<(u64, u128)> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let mtime = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok((meta.len(), mtime))
}

fn entry_for(path: &Path, bytes: &[u8]) -> Result<Entry> {
    let (size, mtime_ns) = stat(path)?;
    Ok(Entry {
        sha256: sha256(bytes),
        size,
        mtime_ns,
    })
}

fn local_path(root: &Path, relative: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(relative.split('/'));
    path
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

impl RemoteOut {
    /// Staging lives under `cache`; the ledger is loaded from it.
    pub fn new(location: S3Location, config: &S3Config, cache: &Path) -> Result<Self> {
        let client = Arc::new(S3Client::new(&location, config)?);
        let mut staging = cache.join("s3-out").join(&location.bucket);
        staging.extend(location.prefix.split('/').filter(|s| !s.is_empty()));
        let ledger = match std::fs::read(staging.join(LEDGER_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("reading the S3 ledger")?,
            Err(_) => Ledger::new(),
        };
        Ok(Self {
            client,
            location,
            staging,
            ledger: Mutex::new(ledger),
            concurrency: config.concurrency.max(1),
        })
    }

    /// Where commands write; stands in for `paths.out`.
    pub fn staging(&self) -> &Path {
        &self.staging
    }

    pub fn location(&self) -> &S3Location {
        &self.location
    }

    fn save_ledger(&self) -> Result<()> {
        let ledger = self.ledger.lock().expect("ledger lock");
        write(
            &self.staging.join(LEDGER_FILE),
            &serde_json::to_vec_pretty(&*ledger)?,
        )
    }

    /// True when the object exists in the bucket as far as the ledger knows.
    pub fn has(&self, relative: &str) -> bool {
        self.ledger
            .lock()
            .expect("ledger lock")
            .contains_key(relative)
    }

    /// Fetches `relative` into staging when it is missing locally and present remotely.
    pub async fn hydrate_file(&self, relative: &str) -> Result<bool> {
        let path = local_path(&self.staging, relative);
        if path.exists() {
            return Ok(true);
        }
        let Some(bytes) = self.client.get(relative).await? else {
            return Ok(false);
        };
        write(&path, &bytes)?;
        let entry = entry_for(&path, &bytes)?;
        self.ledger
            .lock()
            .expect("ledger lock")
            .insert(relative.to_owned(), Some(entry));
        Ok(true)
    }

    /// For every bundle without a local record: fetches the remote record and its JSON files, and
    /// marks its other files as present remotely. Returns how many bundles were hydrated.
    pub async fn hydrate_bundles(self: &Arc<Self>, bundles: &[String]) -> Result<usize> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let mut tasks = JoinSet::new();
        for bundle in bundles {
            let dir = format!("library/{bundle}");
            if local_path(&self.staging, &format!("{dir}/{RECORD_FILE}")).exists() {
                continue;
            }
            let (this, semaphore) = (Arc::clone(self), Arc::clone(&semaphore));
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                this.hydrate_bundle(&dir).await
            });
        }
        let mut hydrated = 0;
        while let Some(result) = tasks.join_next().await {
            if result?? {
                hydrated += 1;
            }
        }
        self.save_ledger()?;
        Ok(hydrated)
    }

    async fn hydrate_bundle(&self, dir: &str) -> Result<bool> {
        let record_path = format!("{dir}/{RECORD_FILE}");
        let Some(bytes) = self.client.get(&record_path).await? else {
            return Ok(false);
        };
        let record: UnpackRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("remote {record_path} is not a ripper-unpack record"))?;
        for file in record.files.iter().filter(|f| f.path.ends_with(".json")) {
            self.hydrate_file(&format!("{dir}/{}", file.path)).await?;
        }
        // The record goes last: locally, too, a record means a complete bundle.
        let path = local_path(&self.staging, &record_path);
        write(&path, &bytes)?;
        let entry = entry_for(&path, &bytes)?;
        let mut ledger = self.ledger.lock().expect("ledger lock");
        for file in &record.files {
            ledger.entry(format!("{dir}/{}", file.path)).or_insert(None);
        }
        ledger.insert(record_path, Some(entry));
        Ok(true)
    }

    /// Uploads every staged file whose content differs from what the ledger recorded, in the
    /// order that keeps records, indexes and the lock after the files they describe.
    pub async fn publish(self: &Arc<Self>) -> Result<PublishSummary> {
        let mut files = Vec::new();
        collect(&self.staging, &self.staging, &mut files)?;
        files.retain(|f| f != LEDGER_FILE);
        let phase = |f: &String| {
            if f == ripper_format::lock::FILE {
                4
            } else if f.starts_with("episodes/") {
                3
            } else if f.starts_with("library/_index/") {
                2
            } else if f.ends_with(&format!("/{RECORD_FILE}")) {
                1
            } else {
                0
            }
        };
        let mut summary = PublishSummary::default();
        for current in 0..=4 {
            let batch: Vec<String> = files
                .iter()
                .filter(|f| phase(f) == current)
                .cloned()
                .collect();
            self.upload_batch(batch, &mut summary).await?;
            self.save_ledger()?;
        }
        Ok(summary)
    }

    async fn upload_batch(
        self: &Arc<Self>,
        batch: Vec<String>,
        summary: &mut PublishSummary,
    ) -> Result<()> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let mut tasks = JoinSet::new();
        for relative in batch {
            let (this, semaphore) = (Arc::clone(self), Arc::clone(&semaphore));
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                let path = local_path(&this.staging, &relative);
                let (size, mtime_ns) = stat(&path)?;
                let known = this
                    .ledger
                    .lock()
                    .expect("ledger lock")
                    .get(&relative)
                    .cloned()
                    .flatten();
                if known
                    .as_ref()
                    .is_some_and(|e| e.size == size && e.mtime_ns == mtime_ns)
                {
                    return anyhow::Ok(None);
                }
                let bytes = std::fs::read(&path)?;
                let entry = Entry {
                    sha256: sha256(&bytes),
                    size,
                    mtime_ns,
                };
                let unchanged = known.is_some_and(|e| e.sha256 == entry.sha256);
                if !unchanged {
                    this.client.put(&relative, bytes).await?;
                }
                this.ledger
                    .lock()
                    .expect("ledger lock")
                    .insert(relative, Some(entry));
                Ok((!unchanged).then_some(size))
            });
        }
        while let Some(result) = tasks.join_next().await {
            match result?? {
                Some(len) => {
                    summary.uploaded += 1;
                    summary.bytes += len;
                }
                None => summary.unchanged += 1,
            }
        }
        Ok(())
    }
}

/// Every file under `dir`, as `/`-separated paths relative to `root`, in a stable order.
fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut entries: Vec<_> = entries.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, out)?;
        } else {
            let relative = path.strip_prefix(root).expect("under root");
            let parts: Vec<_> = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(parts.join("/"));
        }
    }
    Ok(())
}
