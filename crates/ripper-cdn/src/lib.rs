//! CN CDN client.
//!
//! M0 only carries the two pure transforms the spike needs: bundle deobfuscation and manifest
//! decryption. Version lookup, downloading and caching arrive in M1.

pub mod manifest;
pub mod obfuscation;

pub use manifest::{BundleEntry, Manifest, ManifestKey};
pub use obfuscation::deobfuscate;
