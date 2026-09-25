//! JP server access: the CDN only serves a client holding CloudFront signed cookies, which the
//! game obtains with a logged-in account. This does what a fresh install does (JP 6.8.1,
//! `Sekai.UserAccountManager`):
//!
//! 1. `GET {version_api}` → `{profile, assetbundleHostHash, domain}` (asset hosts, API domain);
//! 2. `POST {api}user` (`PostUserAPI`, `{platform, deviceModel, operatingSystem}`) →
//!    `{userRegistration{userId}, credential}`, once; the guest account is kept in the cache;
//! 3. `PUT {api}user/{userId}/auth?refreshUpdatedResources=False` (`PutUserAuthAPI`,
//!    `{credential, deviceId, authTriggerType}`) → `sessionToken`, `assetVersion`, `assetHash`;
//! 4. `POST {signed_cookie_url}` with the session token (`PostSignedCookieAuthAPI`) →
//!    `Set-Cookie: CloudFront-Policy=…; CloudFront-Signature=…; CloudFront-Key-Pair-Id=…`.
//!
//! Every body is AES-128-CBC/PKCS#7 around msgpack with `APIManager.dummydata`/`stab`; JP has one
//! key pair for the API and the manifest, supplied by the user like the CN manifest key (ADR-0005).

use std::path::{Path, PathBuf};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::config::{CdnConfig, fill};
use crate::manifest::{self, ManifestKey};

#[derive(Debug, thiserror::Error)]
pub enum JpError {
    #[error("{step}: {url}: HTTP {status}")]
    Status {
        step: &'static str,
        url: String,
        status: StatusCode,
    },
    #[error("{step}: {url}: {source}")]
    Http {
        step: &'static str,
        url: String,
        source: reqwest::Error,
    },
    #[error("{step}: response is not what the client expects: {reason}")]
    Response { step: &'static str, reason: String },
    #[error("guest account file {path}: {reason}")]
    Account { path: PathBuf, reason: String },
    #[error("no signed cookie in the signature response")]
    NoCookie,
}

/// The guest account, stored as JSON next to the cache (never in a repository).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub install_id: String,
    pub user_id: u64,
    pub credential: String,
}

/// What a login yields: everything the manifest and bundle downloads need.
#[derive(Debug, Clone)]
pub struct Session {
    /// `Cookie` header value for the CDN.
    pub cookie: String,
    pub bundle_host: String,
    pub info_host: String,
    pub asset_version: String,
    pub asset_hash: String,
    pub data_version: String,
    pub user_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    profile: String,
    assetbundle_host_hash: String,
    domain: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserRequest<'a> {
    platform: &'a str,
    device_model: &'a str,
    operating_system: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserRegistration {
    user_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserResponse {
    user_registration: UserRegistration,
    credential: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthRequest<'a> {
    credential: &'a str,
    device_id: Option<&'a str>,
    /// `AuthTriggerType.normal`.
    auth_trigger_type: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    session_token: String,
    asset_version: String,
    asset_hash: String,
    data_version: String,
}

/// A random RFC 4122 v4 UUID (`X-Install-Id`).
fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("OS random source");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The `CloudFront-*=value` pairs of a `Set-Cookie` header (all three arrive in one header,
/// separated by `; `).
pub fn signed_cookie(set_cookie: &str) -> Option<String> {
    let pairs: Vec<&str> = set_cookie
        .split([';', ','])
        .map(str::trim)
        .filter(|part| part.starts_with("CloudFront-") && part.contains('='))
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

pub struct Login<'a> {
    pub http: &'a reqwest::Client,
    pub config: &'a CdnConfig,
    pub key: &'a ManifestKey,
    pub account_file: &'a Path,
}

impl Login<'_> {
    fn headers(
        &self,
        install_id: &str,
        versions: Option<(&str, &str)>,
    ) -> reqwest::header::HeaderMap {
        let c = self.config;
        let (asset, data) = versions.unwrap_or(("", ""));
        let mut pairs = vec![
            ("content-type", "application/octet-stream".to_owned()),
            ("accept", "application/octet-stream".to_owned()),
            ("x-platform", "iOS".to_owned()),
            ("x-devicemodel", c.jp.device_model.clone()),
            ("x-operatingsystem", c.jp.operating_system.clone()),
            ("x-app-version", c.app_version.clone()),
            ("x-app-hash", c.jp.app_hash.clone()),
            ("x-request-id", uuid_v4()),
            ("x-install-id", install_id.to_owned()),
        ];
        if !asset.is_empty() {
            pairs.push(("x-asset-version", asset.to_owned()));
            pairs.push(("x-data-version", data.to_owned()));
        }
        pairs
            .into_iter()
            .filter_map(|(name, value)| Some((name, value.parse().ok()?)))
            .map(|(name, value)| (reqwest::header::HeaderName::from_static(name), value))
            .collect()
    }

    async fn send(
        &self,
        step: &'static str,
        request: reqwest::RequestBuilder,
        url: &str,
    ) -> Result<reqwest::Response, JpError> {
        let response = request.send().await.map_err(|source| JpError::Http {
            step,
            url: url.to_owned(),
            source,
        })?;
        if response.status() != StatusCode::OK {
            return Err(JpError::Status {
                step,
                url: url.to_owned(),
                status: response.status(),
            });
        }
        Ok(response)
    }

    async fn decode<T: for<'de> Deserialize<'de>>(
        &self,
        step: &'static str,
        response: reqwest::Response,
    ) -> Result<T, JpError> {
        let url = response.url().to_string();
        let body = response
            .bytes()
            .await
            .map_err(|source| JpError::Http { step, url, source })?;
        let plain = manifest::decrypt(&body, self.key).map_err(|error| JpError::Response {
            step,
            reason: error.to_string(),
        })?;
        rmp_serde::from_slice(&plain).map_err(|error| JpError::Response {
            step,
            reason: error.to_string(),
        })
    }

    fn body(&self, value: &impl Serialize) -> Vec<u8> {
        let plain = rmp_serde::to_vec_named(value).expect("request structs serialize");
        manifest::encrypt(&plain, self.key)
    }

    fn load_account(&self) -> Result<Option<Account>, JpError> {
        let error = |reason: String| JpError::Account {
            path: self.account_file.to_owned(),
            reason,
        };
        match std::fs::read(self.account_file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| error(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(e.to_string())),
        }
    }

    fn save_account(&self, account: &Account) -> Result<(), JpError> {
        let error = |reason: String| JpError::Account {
            path: self.account_file.to_owned(),
            reason,
        };
        if let Some(dir) = self.account_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| error(e.to_string()))?;
        }
        let json = serde_json::to_vec_pretty(account).map_err(|e| error(e.to_string()))?;
        std::fs::write(self.account_file, json).map_err(|e| error(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(self.account_file, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Registers a guest account (once) and logs in; returns the CDN session.
    pub async fn run(&self) -> Result<Session, JpError> {
        let c = self.config;
        let version_url = fill(
            &c.jp.version_api,
            &[("app", &c.app_version), ("app_hash", &c.jp.app_hash)],
        );
        let response = self
            .send("version", self.http.get(&version_url), &version_url)
            .await?;
        let version: VersionResponse = self.decode("version", response).await?;
        let hosts = [
            ("profile", version.profile.as_str()),
            ("host_hash", version.assetbundle_host_hash.as_str()),
        ];
        let api = format!("https://{}/api/", version.domain);

        let account = match self.load_account()? {
            Some(account) => account,
            None => {
                let install_id = uuid_v4();
                let url = format!("{api}user");
                let request = UserRequest {
                    platform: "iOS",
                    device_model: &c.jp.device_model,
                    operating_system: &c.jp.operating_system,
                };
                let response = self
                    .send(
                        "register",
                        self.http
                            .post(&url)
                            .headers(self.headers(&install_id, None))
                            .body(self.body(&request)),
                        &url,
                    )
                    .await?;
                let user: UserResponse = self.decode("register", response).await?;
                let account = Account {
                    install_id,
                    user_id: user.user_registration.user_id,
                    credential: user.credential,
                };
                self.save_account(&account)?;
                account
            }
        };

        let url = format!(
            "{api}user/{}/auth?refreshUpdatedResources=False",
            account.user_id
        );
        let request = AuthRequest {
            credential: &account.credential,
            device_id: None,
            auth_trigger_type: "normal",
        };
        let response = self
            .send(
                "auth",
                self.http
                    .put(&url)
                    .headers(self.headers(&account.install_id, None))
                    .body(self.body(&request)),
                &url,
            )
            .await?;
        let auth: AuthResponse = self.decode("auth", response).await?;

        let url = &c.jp.signed_cookie_url;
        let response = self
            .send(
                "signature",
                self.http
                    .post(url)
                    .headers(self.headers(
                        &account.install_id,
                        Some((&auth.asset_version, &auth.data_version)),
                    ))
                    .header("x-session-token", &auth.session_token),
                url,
            )
            .await?;
        let cookie = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(signed_cookie)
            .collect::<Vec<_>>()
            .join("; ");
        if cookie.is_empty() {
            return Err(JpError::NoCookie);
        }
        Ok(Session {
            cookie,
            bundle_host: fill(&c.jp.bundle_host, &hosts),
            info_host: fill(&c.jp.info_host, &hosts),
            asset_version: auth.asset_version,
            asset_hash: auth.asset_hash,
            data_version: auth.data_version,
            user_id: account.user_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_one_line_cloudfront_cookie() {
        let header =
            "CloudFront-Policy=eyJ9_; CloudFront-Signature=Mh~G-6__; CloudFront-Key-Pair-Id=KV6QG;";
        assert_eq!(
            signed_cookie(header).unwrap(),
            "CloudFront-Policy=eyJ9_; CloudFront-Signature=Mh~G-6__; CloudFront-Key-Pair-Id=KV6QG"
        );
        assert!(signed_cookie("session=1; Path=/").is_none());
    }

    #[test]
    fn install_ids_are_v4_uuids() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, uuid_v4());
    }
}
