//! Parallel bundle fetching into the cache.
//!
//! Each bundle is: cache lookup → download (length checked by the client) → caller-supplied
//! verification (e.g. the manifest CRC over the decompressed entries, which needs a Unity reader and
//! therefore lives outside this crate) → atomic cache write. Failures are reported per bundle and
//! never abort the others.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::BundleCache;
use crate::client::CdnClient;
use crate::manifest::BundleEntry;

/// Checks a downloaded UnityFS before it is cached; returns a reason on failure.
pub type Verifier = Arc<dyn Fn(&BundleEntry, &[u8]) -> Result<(), String> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchStatus {
    Cached,
    Downloaded { bytes: u64 },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    pub name: String,
    pub status: FetchStatus,
}

async fn fetch_one(
    client: &CdnClient,
    cache: &BundleCache,
    entry: BundleEntry,
    verify: Option<Verifier>,
) -> FetchStatus {
    match cache.contains(&entry) {
        Ok(true) => return FetchStatus::Cached,
        Ok(false) => {}
        Err(error) => return FetchStatus::Failed(error.to_string()),
    }
    let data = match client.bundle(&entry).await {
        Ok(data) => data,
        Err(error) => return FetchStatus::Failed(error.to_string()),
    };
    let checked = tokio::task::spawn_blocking(move || {
        if let Some(verify) = verify {
            verify(&entry, &data)?;
        }
        Ok::<_, String>((entry, data))
    })
    .await;
    let (entry, data) = match checked {
        Ok(Ok(checked)) => checked,
        Ok(Err(reason)) => return FetchStatus::Failed(format!("verification failed: {reason}")),
        Err(error) => return FetchStatus::Failed(format!("verification panicked: {error}")),
    };
    match cache.put(&entry, &data) {
        Ok(_) => FetchStatus::Downloaded {
            bytes: data.len() as u64,
        },
        Err(error) => FetchStatus::Failed(error.to_string()),
    }
}

/// Fetches `entries` with at most `client.config().concurrency` downloads in flight.
/// `on_done` is called as each bundle finishes (for progress output); results are returned in
/// completion order.
pub async fn fetch_all(
    client: Arc<CdnClient>,
    cache: Arc<BundleCache>,
    entries: Vec<BundleEntry>,
    verify: Option<Verifier>,
    mut on_done: impl FnMut(&FetchOutcome),
) -> Vec<FetchOutcome> {
    let permits = Arc::new(Semaphore::new(client.config().concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for entry in entries {
        let (client, cache, verify, permits) = (
            client.clone(),
            cache.clone(),
            verify.clone(),
            permits.clone(),
        );
        tasks.spawn(async move {
            let _permit = permits
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let name = entry.bundle_name.clone();
            FetchOutcome {
                name,
                status: fetch_one(&client, &cache, entry, verify).await,
            }
        });
    }
    let mut outcomes = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let outcome = joined.unwrap_or_else(|error| FetchOutcome {
            name: "<task>".into(),
            status: FetchStatus::Failed(error.to_string()),
        });
        on_done(&outcome);
        outcomes.push(outcome);
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{config_for, serve};
    use std::collections::HashMap;

    fn entry(name: &str, file_size: u64) -> BundleEntry {
        BundleEntry {
            bundle_name: name.into(),
            category: None,
            file_size,
            dependencies: vec![],
            download_path: "ios1".into(),
            crc: 7,
        }
    }

    #[tokio::test]
    async fn downloads_verifies_caches_and_skips_cached() {
        let plain = b"UnityFS\0payload".to_vec();
        let mut downloaded = vec![0x10, 0, 0, 0];
        downloaded.extend(crate::obfuscation::tests_support::obfuscate_body(&plain));
        let root = "/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios1";
        let routes = HashMap::from([
            (format!("{root}/good/one"), (200, downloaded.clone())),
            (format!("{root}/bad/two"), (200, downloaded)),
        ]);
        let (host, hits) = serve(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let client = Arc::new(CdnClient::new(config_for(vec![host]), None).unwrap());
        let cache = Arc::new(BundleCache::new(dir.path()));
        let verify: Verifier = Arc::new(|entry, _| {
            if entry.bundle_name.starts_with("bad") {
                Err("crc".into())
            } else {
                Ok(())
            }
        });
        let entries = vec![
            entry("good/one", plain.len() as u64),
            entry("bad/two", plain.len() as u64),
            entry("missing/three", 1),
        ];

        let mut outcomes = fetch_all(
            client.clone(),
            cache.clone(),
            entries.clone(),
            Some(verify.clone()),
            |_| {},
        )
        .await;
        outcomes.sort_by(|a, b| a.name.cmp(&b.name));
        assert!(
            matches!(&outcomes[0].status, FetchStatus::Failed(r) if r.contains("verification"))
        );
        assert_eq!(
            outcomes[1].status,
            FetchStatus::Downloaded {
                bytes: plain.len() as u64
            }
        );
        assert!(matches!(&outcomes[2].status, FetchStatus::Failed(r) if r.contains("not found")));
        assert_eq!(
            cache.get(&entries[0]).unwrap().as_deref(),
            Some(plain.as_slice())
        );
        assert!(!cache.contains(&entries[1]).unwrap());

        let before = hits.load(std::sync::atomic::Ordering::SeqCst);
        let again = fetch_all(
            client,
            cache,
            vec![entries[0].clone()],
            Some(verify),
            |_| {},
        )
        .await;
        assert_eq!(again[0].status, FetchStatus::Cached);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), before);
    }
}
