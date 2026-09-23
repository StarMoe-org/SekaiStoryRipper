//! CN CDN client: asset version, manifest (decrypt, archive, diff), bundle download and cache.
//!
//! Everything here is anonymous HTTPS GET (see `Sekai/ASSET_DOWNLOAD_GUIDE.md`). The only secret
//! involved, the ABCrypt manifest key, is supplied by the user (decision D4).

pub mod cache;
pub mod client;
pub mod config;
pub mod diff;
pub mod download;
pub mod manifest;
pub mod obfuscation;
pub mod portable;
pub mod store;

pub use cache::BundleCache;
pub use client::{CdnClient, CdnError};
pub use config::CdnConfig;
pub use diff::ManifestDiff;
pub use manifest::{BundleEntry, Manifest, ManifestKey};
pub use obfuscation::deobfuscate;
pub use store::ManifestStore;
