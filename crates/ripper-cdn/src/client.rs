//! HTTPS client for the asset CDN: anonymous for CN; for JP, every request carries the signed
//! cookie of a guest login (see [`crate::jp`]), made once per client on first use.
//!
//! Transient failures (connect/timeout errors, 5xx, short bodies) are retried with exponential
//! backoff, moving to the next equivalent host each time. A 404 is not retried: on this CDN it
//! means the `downloadPath` or app version in the URL is wrong, and another host will say the same.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use tokio::sync::OnceCell;

use crate::config::{CdnConfig, Region};
use crate::jp::{self, JpError, Session};
use crate::manifest::{self, BundleEntry, ManifestError, ManifestKey};
use crate::obfuscation::deobfuscate;

/// An asset version as the server names it, with the JP asset hash that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetVersion {
    /// CN `N` of `ios{N}`; JP e.g. `6.8.0.50`.
    pub version: String,
    /// JP only.
    pub hash: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CdnError {
    #[error("{url}: not found (wrong downloadPath or app version?)")]
    NotFound { url: String },
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: StatusCode },
    #[error("{url}: {source}")]
    Http { url: String, source: reqwest::Error },
    #[error("{url}: got {got} bytes, expected {expected}")]
    Length {
        url: String,
        got: u64,
        expected: u64,
    },
    #[error("{url}: version file is not a number: {body:?}")]
    VersionFormat { url: String, body: String },
    #[error(
        "no ABCrypt key configured; set RIPPER_AB_KEY/RIPPER_AB_IV or [crypto] in the config (ADR-0005)"
    )]
    MissingKey,
    #[error("JP login: {0}")]
    Jp(#[from] JpError),
    #[error("JP needs an account file (the client was built without one)")]
    NoAccountFile,
    #[error("JP asset version {0} has no asset hash; pin cdn.jp.asset_hash or omit the version")]
    MissingAssetHash(String),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("TLS setup failed: {0}")]
    Tls(String),
    #[error("no CDN hosts configured")]
    NoHosts,
}

impl CdnError {
    fn is_transient(&self) -> bool {
        match self {
            Self::Http { .. } | Self::Length { .. } => true,
            Self::Status { status, .. } => {
                status.is_server_error() || *status == StatusCode::TOO_MANY_REQUESTS
            }
            _ => false,
        }
    }
}

pub struct CdnClient {
    http: reqwest::Client,
    config: CdnConfig,
    key: Option<ManifestKey>,
    /// JP guest account (created on first login).
    account_file: Option<PathBuf>,
    session: OnceCell<Session>,
}

/// rustls with the `ring` provider and the bundled webpki roots (ADR-0010); shared by every
/// HTTP client the tool builds.
pub fn tls_config() -> Result<rustls::ClientConfig, CdnError> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|error| CdnError::Tls(error.to_string()))
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

impl CdnClient {
    pub fn new(config: CdnConfig, key: Option<ManifestKey>) -> Result<Self, CdnError> {
        if config.region == Region::Cn && config.hosts.is_empty() {
            return Err(CdnError::NoHosts);
        }
        let mut headers = reqwest::header::HeaderMap::new();
        let unity = reqwest::header::HeaderValue::from_str(&config.unity_version)
            .map_err(|error| CdnError::Tls(format!("bad x-unity-version header: {error}")))?;
        headers.insert("x-unity-version", unity);
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(tls_config()?)
            .user_agent(config.user_agent.clone())
            .default_headers(headers)
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|source| CdnError::Http {
                url: "<client>".into(),
                source,
            })?;
        Ok(Self {
            http,
            config,
            key,
            account_file: None,
            session: OnceCell::new(),
        })
    }

    /// Where the JP guest account is kept (required for JP).
    pub fn with_account_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.account_file = Some(path.into());
        self
    }

    pub fn config(&self) -> &CdnConfig {
        &self.config
    }

    /// The JP login, made on first use and shared by every later request.
    pub async fn session(&self) -> Result<&Session, CdnError> {
        self.session
            .get_or_try_init(|| async {
                let key = self.key.as_ref().ok_or(CdnError::MissingKey)?;
                let account_file = self
                    .account_file
                    .as_deref()
                    .ok_or(CdnError::NoAccountFile)?;
                let login = jp::Login {
                    http: &self.http,
                    config: &self.config,
                    key,
                    account_file,
                };
                Ok(login.run().await?)
            })
            .await
    }

    /// Hosts and cookie for bundle downloads.
    async fn bundle_hosts(&self) -> Result<(Vec<String>, Option<String>), CdnError> {
        match self.config.region {
            Region::Cn => Ok((self.config.hosts.clone(), None)),
            Region::Jp => {
                let session = self.session().await?;
                Ok((
                    vec![session.bundle_host.clone()],
                    Some(session.cookie.clone()),
                ))
            }
        }
    }

    /// GETs a URL built for each host in turn, retrying transient failures.
    async fn get(
        &self,
        hosts: &[String],
        cookie: Option<&str>,
        url_for: impl Fn(&str) -> String,
        expected_len: Option<u64>,
    ) -> Result<Vec<u8>, CdnError> {
        if hosts.is_empty() {
            return Err(CdnError::NoHosts);
        }
        let mut attempt = 0u32;
        loop {
            let url = url_for(&hosts[attempt as usize % hosts.len()]);
            let result = self.get_once(&url, cookie, expected_len).await;
            match result {
                Err(error) if error.is_transient() && attempt < self.config.retries => {
                    tokio::time::sleep(Duration::from_millis(500 << attempt.min(6))).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    async fn get_once(
        &self,
        url: &str,
        cookie: Option<&str>,
        expected_len: Option<u64>,
    ) -> Result<Vec<u8>, CdnError> {
        let http = |source| CdnError::Http {
            url: url.to_owned(),
            source,
        };
        let mut request = self.http.get(url);
        if let Some(cookie) = cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let response = request.send().await.map_err(http)?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => {
                return Err(CdnError::NotFound {
                    url: url.to_owned(),
                });
            }
            status => {
                return Err(CdnError::Status {
                    url: url.to_owned(),
                    status,
                });
            }
        }
        // Without a manifest size, the transfer itself must at least be complete.
        let expected_len = expected_len.or(response.content_length());
        let body = response.bytes().await.map_err(http)?;
        if let Some(expected) = expected_len
            && body.len() as u64 != expected
        {
            // A truncated body still parses as UnityFS and only fails later in LZ4, so check here.
            return Err(CdnError::Length {
                url: url.to_owned(),
                got: body.len() as u64,
                expected,
            });
        }
        Ok(body.to_vec())
    }

    /// GETs an arbitrary URL once per configured retry (used for masterdata, which is not on the CDN).
    pub async fn get_url(&self, url: &str) -> Result<Vec<u8>, CdnError> {
        let mut attempt = 0u32;
        loop {
            match self.get_once(url, None, None).await {
                Err(error) if error.is_transient() && attempt < self.config.retries => {
                    tokio::time::sleep(Duration::from_millis(500 << attempt.min(6))).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// The current asset version, unless pinned in the config. CN reads the CDN `version` file;
    /// JP logs in and takes `assetVersion`/`assetHash` from the auth response.
    pub async fn asset_version(&self) -> Result<AssetVersion, CdnError> {
        match self.config.region {
            Region::Cn => self.cn_asset_version().await,
            Region::Jp => {
                if let Some(version) = &self.config.asset_version {
                    let hash = self
                        .config
                        .jp
                        .asset_hash
                        .clone()
                        .ok_or_else(|| CdnError::MissingAssetHash(version.clone()))?;
                    return Ok(AssetVersion {
                        version: version.clone(),
                        hash: Some(hash),
                    });
                }
                let session = self.session().await?;
                Ok(AssetVersion {
                    version: session.asset_version.clone(),
                    hash: Some(session.asset_hash.clone()),
                })
            }
        }
    }

    async fn cn_asset_version(&self) -> Result<AssetVersion, CdnError> {
        if let Some(version) = &self.config.asset_version {
            return Ok(AssetVersion {
                version: version.clone(),
                hash: None,
            });
        }
        let buster = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let body = self
            .get(
                &self.config.hosts,
                None,
                |host| self.config.version_url(host, buster),
                None,
            )
            .await?;
        let text = String::from_utf8_lossy(&body).trim().to_owned();
        match text.parse::<u32>() {
            Ok(version) => Ok(AssetVersion {
                version: version.to_string(),
                hash: None,
            }),
            Err(_) => Err(CdnError::VersionFormat {
                url: self.config.version_url(&self.config.hosts[0], buster),
                body: text,
            }),
        }
    }

    /// Downloads and decrypts the manifest (CN `ios{N}/AssetBundleInfoNew.json`, JP the
    /// assetbundle-info API), returning the msgpack plaintext.
    pub async fn manifest_plain(&self, asset: &AssetVersion) -> Result<Vec<u8>, CdnError> {
        let key = self.key.as_ref().ok_or(CdnError::MissingKey)?;
        let ciphertext = match self.config.region {
            Region::Cn => {
                self.get(
                    &self.config.hosts,
                    None,
                    |host| self.config.manifest_url(host, &asset.version),
                    None,
                )
                .await?
            }
            Region::Jp => {
                let hash = asset
                    .hash
                    .as_deref()
                    .ok_or_else(|| CdnError::MissingAssetHash(asset.version.clone()))?;
                let session = self.session().await?;
                self.get(
                    std::slice::from_ref(&session.info_host),
                    Some(&session.cookie),
                    |host| self.config.jp_manifest_url(host, &asset.version, hash),
                    None,
                )
                .await?
            }
        };
        Ok(manifest::decrypt(&ciphertext, key)?)
    }

    /// Downloads one bundle from its own `downloadPath` and returns the deobfuscated UnityFS.
    ///
    /// CN bundles must be exactly `fileSize + 4` bytes. JP bundles are only checked against their
    /// `Content-Length`: the JP CDN serves some bundles that differ from their manifest entry, and
    /// the game accepts them as they are (`AssetBundleDownloadHandler` uses `fileSize` only to
    /// size its buffer and checks neither size nor CRC), so they are the assets the game shows.
    pub async fn bundle(&self, entry: &BundleEntry) -> Result<Vec<u8>, CdnError> {
        let (hosts, cookie) = self.bundle_hosts().await?;
        let expected = match self.config.region {
            Region::Cn => Some(entry.file_size + 4),
            Region::Jp => None,
        };
        let raw = self
            .get(
                &hosts,
                cookie.as_deref(),
                |host| {
                    self.config
                        .bundle_url(host, &entry.download_path, &entry.bundle_name)
                },
                expected,
            )
            .await?;
        Ok(deobfuscate(raw))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal HTTP/1.1 server: path -> (status, body). Counts requests per path.
    pub(crate) async fn serve(
        routes: HashMap<String, (u16, Vec<u8>)>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let routes = routes.clone();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let n = socket.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..n]);
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .split('?')
                        .next()
                        .unwrap_or("/")
                        .to_owned();
                    counter.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = routes
                        .get(&path)
                        .cloned()
                        .unwrap_or((404, b"{\"Success\":-1}".to_vec()));
                    let head = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        (address, hits)
    }

    pub(crate) fn config_for(hosts: Vec<String>) -> CdnConfig {
        CdnConfig {
            hosts,
            retries: 2,
            timeout_secs: 5,
            ..CdnConfig::default()
        }
    }

    fn entry(name: &str, file_size: u64) -> BundleEntry {
        BundleEntry {
            bundle_name: name.into(),
            category: None,
            file_size,
            dependencies: vec![],
            download_path: "ios1".into(),
            crc: 0,
        }
    }

    fn obfuscated(plain: &[u8]) -> Vec<u8> {
        let mut out = vec![0x10, 0, 0, 0];
        out.extend(crate::obfuscation::tests_support::obfuscate_body(plain));
        out
    }

    fn bundle_path(name: &str) -> String {
        format!("/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios1/{name}")
    }

    #[tokio::test]
    async fn reads_version_and_downloads_a_bundle() {
        let plain: Vec<u8> = b"UnityFS\0".iter().copied().chain(0..200u8).collect();
        let routes = HashMap::from([
            (
                "/obj/rt-game-lf/gdl_app_5236/Mainland/6.4.0/Release/cn_online/ios/version"
                    .to_owned(),
                (200, b"10\n".to_vec()),
            ),
            (bundle_path("a/b"), (200, obfuscated(&plain))),
        ]);
        let (host, _) = serve(routes).await;
        let client = CdnClient::new(config_for(vec![host]), None).unwrap();
        assert_eq!(client.asset_version().await.unwrap().version, "10");
        assert_eq!(
            client
                .bundle(&entry("a/b", plain.len() as u64))
                .await
                .unwrap(),
            plain
        );
    }

    #[tokio::test]
    async fn not_found_fails_without_retrying() {
        let (host, hits) = serve(HashMap::new()).await;
        let client = CdnClient::new(config_for(vec![host]), None).unwrap();
        assert!(matches!(
            client.bundle(&entry("x/y", 1)).await,
            Err(CdnError::NotFound { .. })
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_short_body_is_retried_on_the_next_host_then_reported() {
        let routes = HashMap::from([(bundle_path("a/b"), (200, vec![0u8; 10]))]);
        let (first, hits_first) = serve(routes.clone()).await;
        let (second, hits_second) = serve(routes).await;
        let client = CdnClient::new(config_for(vec![first, second]), None).unwrap();
        let result = client.bundle(&entry("a/b", 100)).await;
        assert!(matches!(
            result,
            Err(CdnError::Length {
                got: 10,
                expected: 104,
                ..
            })
        ));
        // 1 try + 2 retries, alternating hosts.
        assert_eq!(
            hits_first.load(Ordering::SeqCst) + hits_second.load(Ordering::SeqCst),
            3
        );
        assert!(hits_second.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn manifest_requires_a_user_supplied_key() {
        let client = CdnClient::new(config_for(vec!["http://127.0.0.1:9".into()]), None).unwrap();
        let asset = AssetVersion {
            version: "10".into(),
            hash: None,
        };
        assert!(matches!(
            client.manifest_plain(&asset).await,
            Err(CdnError::MissingKey)
        ));
    }
}
