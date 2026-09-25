//! Minimal S3 client (ADR-0012): SigV4 presigned requests from `rusty-s3`, sent with the same
//! reqwest + rustls/ring stack as the CDN client. Works with AWS S3 and S3-compatible services
//! (MinIO, Cloudflare R2, ...).
//!
//! Credentials come from the environment only (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
//! optional `AWS_SESSION_TOKEN`). Endpoint, region and addressing style come from `[s3]` in the
//! config, else `AWS_ENDPOINT_URL_S3` / `AWS_ENDPOINT_URL`, `AWS_REGION` / `AWS_DEFAULT_REGION`
//! and `S3_ADDRESSING_STYLE`.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::{Deserialize, Serialize};

/// How long a presigned request stays valid; requests are sent right after signing.
const SIGNATURE_TTL: Duration = Duration::from_secs(15 * 60);

/// `[s3]`: where S3 outputs (`--out s3://bucket/prefix`) go.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct S3Config {
    /// Endpoint URL; unset means AWS (`https://s3.<region>.amazonaws.com`).
    pub endpoint: Option<String>,
    /// Signing region; unset means the environment, else `us-east-1`.
    pub region: Option<String>,
    /// `auto` (path style for a custom endpoint, virtual-hosted for AWS), `path` or `virtual`.
    pub addressing_style: Option<String>,
    /// Parallel uploads / downloads.
    pub concurrency: usize,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            endpoint: None,
            region: None,
            addressing_style: None,
            concurrency: 16,
        }
    }
}

/// `s3://bucket/prefix` split into its parts; the prefix has no leading or trailing `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Location {
    pub bucket: String,
    pub prefix: String,
}

impl S3Location {
    /// `Some` for an `s3://` URL, `None` for anything else (a local path).
    pub fn parse(text: &str) -> Option<Result<Self>> {
        let rest = text.strip_prefix("s3://")?;
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        Some(if bucket.is_empty() {
            Err(anyhow::anyhow!("{text}: missing bucket name"))
        } else {
            Ok(Self {
                bucket: bucket.to_owned(),
                prefix: prefix.trim_matches('/').to_owned(),
            })
        })
    }
}

impl std::fmt::Display for S3Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s3://{}/{}", self.bucket, self.prefix)
    }
}

fn env(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

pub struct S3Client {
    http: reqwest::Client,
    bucket: Bucket,
    credentials: Credentials,
    prefix: String,
}

impl S3Client {
    pub fn new(location: &S3Location, config: &S3Config) -> Result<Self> {
        let (Some(key), Some(secret)) =
            (env(&["AWS_ACCESS_KEY_ID"]), env(&["AWS_SECRET_ACCESS_KEY"]))
        else {
            bail!("S3 output needs AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY in the environment");
        };
        let credentials = match env(&["AWS_SESSION_TOKEN"]) {
            Some(token) => Credentials::new_with_token(key, secret, token),
            None => Credentials::new(key, secret),
        };
        let region = config
            .region
            .clone()
            .or_else(|| env(&["AWS_REGION", "AWS_DEFAULT_REGION"]))
            .unwrap_or_else(|| "us-east-1".into());
        let custom = config
            .endpoint
            .clone()
            .or_else(|| env(&["AWS_ENDPOINT_URL_S3", "AWS_ENDPOINT_URL"]));
        let style = config
            .addressing_style
            .clone()
            .or_else(|| env(&["S3_ADDRESSING_STYLE"]))
            .unwrap_or_else(|| "auto".into());
        let style = match style.as_str() {
            "path" => UrlStyle::Path,
            "virtual" => UrlStyle::VirtualHost,
            "auto" if custom.is_some() => UrlStyle::Path,
            "auto" => UrlStyle::VirtualHost,
            other => bail!("s3.addressing_style must be auto, path or virtual (got {other:?})"),
        };
        let endpoint = custom.unwrap_or_else(|| format!("https://s3.{region}.amazonaws.com"));
        let endpoint: url::Url = endpoint
            .parse()
            .with_context(|| format!("bad S3 endpoint {endpoint:?}"))?;
        let bucket = Bucket::new(endpoint, style, location.bucket.clone(), region)
            .map_err(|e| anyhow::anyhow!("S3 bucket {}: {e:?}", location.bucket))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(ripper_cdn::client::tls_config()?)
            .timeout(Duration::from_secs(600))
            .build()?;
        Ok(Self {
            http,
            bucket,
            credentials,
            prefix: location.prefix.clone(),
        })
    }

    /// The object key of `relative` (a `/`-separated path under the prefix).
    pub fn key(&self, relative: &str) -> String {
        if self.prefix.is_empty() {
            relative.to_owned()
        } else {
            format!("{}/{relative}", self.prefix)
        }
    }

    /// The object's bytes, or `None` when it does not exist.
    pub async fn get(&self, relative: &str) -> Result<Option<Vec<u8>>> {
        let key = self.key(relative);
        let url = self
            .bucket
            .get_object(Some(&self.credentials), &key)
            .sign(SIGNATURE_TTL);
        let response = self.http.get(url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = checked(response, "GET", &key).await?;
        Ok(Some(response.bytes().await?.to_vec()))
    }

    pub async fn put(&self, relative: &str, bytes: Vec<u8>) -> Result<()> {
        let key = self.key(relative);
        let url = self
            .bucket
            .put_object(Some(&self.credentials), &key)
            .sign(SIGNATURE_TTL);
        let response = self
            .http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type(relative))
            .body(bytes)
            .send()
            .await?;
        checked(response, "PUT", &key).await?;
        Ok(())
    }
}

async fn checked(
    response: reqwest::Response,
    method: &str,
    key: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    bail!("S3 {method} {key}: {status}: {}", body.trim())
}

/// `Content-Type` by extension, so objects can be served straight from the bucket.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("wav") => "audio/wav",
        Some("otf") => "font/otf",
        Some("ttf") => "font/ttf",
        Some("m2v") => "video/mpeg",
        Some("mp4") => "video/mp4",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_s3_urls() {
        assert_eq!(S3Location::parse("out/jp").map(|r| r.unwrap()), None);
        let loc = S3Location::parse("s3://sekai/library/jp/")
            .unwrap()
            .unwrap();
        assert_eq!(
            (loc.bucket.as_str(), loc.prefix.as_str()),
            ("sekai", "library/jp")
        );
        let root = S3Location::parse("s3://sekai").unwrap().unwrap();
        assert_eq!(root.prefix, "");
        assert!(S3Location::parse("s3:///x").unwrap().is_err());
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type("a/b/_ripper.json"), "application/json");
        assert_eq!(content_type("x.WAV"), "audio/wav");
        assert_eq!(content_type("x.moc3"), "application/octet-stream");
    }
}
