//! Asset CDN client (CN and JP): asset version, manifest (decrypt, archive, diff), bundle download
//! and cache.
//!
//! CN is anonymous HTTPS GET (see `Sekai/ASSET_DOWNLOAD_GUIDE.md`); JP needs a guest login for its
//! signed cookies (see [`jp`]). The only secret involved, the AES key of the manifest (and, for JP,
//! of the API), is supplied by the user (ADR-0005).

pub mod cache;
pub mod client;
pub mod config;
pub mod diff;
pub mod download;
pub mod jp;
pub mod manifest;
pub mod obfuscation;
pub mod store;

pub use cache::BundleCache;
pub use client::{AssetVersion, CdnClient, CdnError};
pub use config::{CdnConfig, JpConfig, Region};
pub use diff::ManifestDiff;
pub use manifest::{BundleEntry, Manifest, ManifestKey};
pub use obfuscation::deobfuscate;
pub use store::{ManifestMeta, ManifestStore};
