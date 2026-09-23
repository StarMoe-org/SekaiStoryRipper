//! `ripper.toml`: defaults ← config file ← environment ← command-line flags.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ripper_cdn::{CdnConfig, ManifestKey};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG_FILE: &str = "ripper.toml";
pub const ENV_AB_KEY: &str = "RIPPER_AB_KEY";
pub const ENV_AB_IV: &str = "RIPPER_AB_IV";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub cdn: CdnConfig,
    pub crypto: CryptoConfig,
    pub masterdata: MasterdataConfig,
    pub paths: PathsConfig,
    pub unity: UnityConfig,
    pub tools: ToolsConfig,
}

/// ABCrypt key/IV for the manifest (decision D4: never shipped, always user-supplied).
/// Each is the 16-character string or 32 hex digits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CryptoConfig {
    pub ab_key: Option<String>,
    pub ab_iv: Option<String>,
}

/// Masterdata source (decision D15): `{name}` is replaced by the table name, e.g. `unitStories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MasterdataConfig {
    pub url_template: String,
}

impl Default for MasterdataConfig {
    fn default() -> Self {
        Self {
            url_template: "https://raw.githubusercontent.com/Team-Haruki/haruki-sekai-sc-master/HEAD/master/{name}.json"
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathsConfig {
    pub cache: PathBuf,
    pub out: PathBuf,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            cache: "cache".into(),
            out: "out".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UnityConfig {
    /// Used for bundles whose header version was stripped (all CN CDN bundles).
    pub version: String,
}

impl Default for UnityConfig {
    fn default() -> Self {
        Self {
            version: ripper_unity::DEFAULT_UNITY_VERSION.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// ffmpeg executable for ADX → WAV (M5). A bare name is looked up on PATH (`ffmpeg.exe` on Windows).
    pub ffmpeg: PathBuf,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl Config {
    /// Loads `path`, or `ripper.toml` in the working directory when present, then applies env.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut config = match path {
            Some(path) => Self::from_file(path)?,
            None if Path::new(DEFAULT_CONFIG_FILE).exists() => {
                Self::from_file(Path::new(DEFAULT_CONFIG_FILE))?
            }
            None => Self::default(),
        };
        if let Ok(key) = std::env::var(ENV_AB_KEY) {
            config.crypto.ab_key = Some(key);
        }
        if let Ok(iv) = std::env::var(ENV_AB_IV) {
            config.crypto.ab_iv = Some(iv);
        }
        Ok(config)
    }

    fn from_file(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// The manifest key, if both parts are configured.
    pub fn manifest_key(&self) -> Result<Option<ManifestKey>> {
        match (&self.crypto.ab_key, &self.crypto.ab_iv) {
            (Some(key), Some(iv)) => Ok(Some(
                ManifestKey::from_text(key, iv).context("invalid ABCrypt key/IV")?,
            )),
            (None, None) => Ok(None),
            _ => anyhow::bail!(
                "set both {ENV_AB_KEY} and {ENV_AB_IV} (or crypto.ab_key and crypto.ab_iv)"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_config_parses_and_matches_the_defaults() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ripper.example.toml");
        let example = Config::from_file(Path::new(path)).unwrap();
        let defaults = Config::default();
        assert_eq!(example.cdn, defaults.cdn);
        assert_eq!(
            example.masterdata.url_template,
            defaults.masterdata.url_template
        );
        assert_eq!(example.unity.version, defaults.unity.version);
        assert!(
            example.crypto.ab_key.is_none(),
            "the example must not carry a key"
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("[cdn]\nhostz = []\n").is_err());
    }
}
