//! `ripper.toml`: region preset ← config file ← environment ← command-line flags.
//!
//! The region (`--region`, else `cdn.region` in the file, else CN) picks the preset every other
//! value defaults to, so a JP config only has to say `region = "jp"`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ripper_cdn::{BundleCache, CdnClient, CdnConfig, ManifestKey, Region};
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

/// ABCrypt key/IV for the manifest (ADR-0005: never shipped, always user-supplied).
/// Each is the 16-character string or 32 hex digits. JP uses one key pair (`APIManager.Crypt`) for
/// the manifest and the login API, so the same two values serve both.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CryptoConfig {
    pub ab_key: Option<String>,
    pub ab_iv: Option<String>,
}

/// Masterdata source (ADR-0009): `{name}` is replaced by the table name, e.g. `unitStories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MasterdataConfig {
    pub url_template: String,
}

impl Default for MasterdataConfig {
    fn default() -> Self {
        Self::preset(Region::Cn)
    }
}

impl MasterdataConfig {
    pub fn preset(region: Region) -> Self {
        let repo = match region {
            Region::Cn => "haruki-sekai-sc-master",
            Region::Jp => "haruki-sekai-master",
        };
        Self {
            url_template: format!(
                "https://raw.githubusercontent.com/Team-Haruki/{repo}/HEAD/master/{{name}}.json"
            ),
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
        Self::preset(Region::Cn)
    }
}

impl PathsConfig {
    /// JP lives in its own subdirectories: bundle names are shared between servers, contents not.
    pub fn preset(region: Region) -> Self {
        match region {
            Region::Cn => Self {
                cache: "cache".into(),
                out: "out".into(),
            },
            Region::Jp => Self {
                cache: "cache/jp".into(),
                out: "out/jp".into(),
            },
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
        Self::preset(Region::Cn)
    }
}

impl UnityConfig {
    pub fn preset(region: Region) -> Self {
        Self {
            version: match region {
                Region::Cn => ripper_unity::DEFAULT_UNITY_VERSION.into(),
                // UnityFramework of the JP 6.8.1 ipa.
                Region::Jp => "2022.3.62f2".into(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// ffmpeg executable for ADX → WAV. A bare name is looked up on PATH (`ffmpeg.exe` on Windows).
    pub ffmpeg: PathBuf,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
        }
    }
}

/// Overlays `top` onto `base`, table by table.
fn merge(base: &mut toml::Table, top: toml::Table) {
    for (key, value) in top {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(top)) => merge(base, top),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

impl Config {
    pub fn preset(region: Region) -> Self {
        Self {
            cdn: CdnConfig::preset(region),
            crypto: CryptoConfig::default(),
            masterdata: MasterdataConfig::preset(region),
            paths: PathsConfig::preset(region),
            unity: UnityConfig::preset(region),
            tools: ToolsConfig::default(),
        }
    }

    /// Loads `path`, or `ripper.toml` in the working directory when present, over the region's
    /// preset, then applies env.
    pub fn load(path: Option<&Path>, region: Option<Region>) -> Result<Self> {
        let file = match path {
            Some(path) => Some(Self::read_table(path)?),
            None if Path::new(DEFAULT_CONFIG_FILE).exists() => {
                Some(Self::read_table(Path::new(DEFAULT_CONFIG_FILE))?)
            }
            None => None,
        };
        let mut config = Self::layered(file, region)?;
        if let Ok(key) = std::env::var(ENV_AB_KEY) {
            config.crypto.ab_key = Some(key);
        }
        if let Ok(iv) = std::env::var(ENV_AB_IV) {
            config.crypto.ab_iv = Some(iv);
        }
        Ok(config)
    }

    fn read_table(path: &Path) -> Result<toml::Table> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// The region's preset with `file` on top; `region` (the command line) wins over the file.
    fn layered(file: Option<toml::Table>, region: Option<Region>) -> Result<Self> {
        let file = file.unwrap_or_default();
        let from_file = file
            .get("cdn")
            .and_then(|cdn| cdn.get("region"))
            .map(|value| value.clone().try_into::<Region>())
            .transpose()
            .context("cdn.region must be \"cn\" or \"jp\"")?;
        let region = region.or(from_file).unwrap_or_default();
        let mut table = toml::Table::try_from(Self::preset(region))?;
        merge(&mut table, file);
        let mut config: Self = table.try_into().context("invalid configuration")?;
        config.cdn.region = region;
        Ok(config)
    }

    #[cfg(test)]
    fn from_file(path: &Path) -> Result<Self> {
        Self::layered(Some(Self::read_table(path)?), None)
    }

    /// The JP guest account file (created by the first JP login).
    pub fn account_file(&self) -> PathBuf {
        self.paths.cache.join("account.json")
    }

    /// The bundle cache; JP entries are not size-checked against the manifest.
    pub fn bundle_cache(&self) -> BundleCache {
        let cache = BundleCache::new(&self.paths.cache);
        match self.cdn.region {
            Region::Cn => cache,
            Region::Jp => cache.without_size_check(),
        }
    }

    /// A CDN client with the configured key (if any) and, for JP, the account file.
    pub fn cdn_client(&self, cdn: CdnConfig) -> Result<CdnClient> {
        let key = self.manifest_key()?;
        if cdn.region == Region::Jp && key.is_none() {
            anyhow::bail!(
                "JP needs the API/manifest key: set {ENV_AB_KEY} and {ENV_AB_IV} (or [crypto])"
            );
        }
        Ok(CdnClient::new(cdn, key)?.with_account_file(self.account_file()))
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
        let file: toml::Table = toml::from_str("[cdn]\nhostz = []\n").unwrap();
        assert!(Config::layered(Some(file), None).is_err());
    }

    #[test]
    fn the_region_picks_the_preset_and_the_file_still_overrides_it() {
        let file: toml::Table =
            toml::from_str("[cdn]\nregion = \"jp\"\nconcurrency = 3\n").unwrap();
        let config = Config::layered(Some(file.clone()), None).unwrap();
        assert_eq!(config.cdn.region, Region::Jp);
        assert_eq!(config.cdn.app_version, "6.8.1");
        assert_eq!(config.cdn.concurrency, 3);
        assert_eq!(config.paths.cache, PathBuf::from("cache/jp"));
        assert!(
            config
                .masterdata
                .url_template
                .contains("/haruki-sekai-master/")
        );
        assert_eq!(config.unity.version, "2022.3.62f2");

        let forced = Config::layered(Some(file), Some(Region::Cn)).unwrap();
        assert_eq!(forced.cdn.region, Region::Cn);
        assert_eq!(forced.cdn.app_version, "6.4.0");
        assert_eq!(
            Config::layered(None, None).unwrap().cdn,
            CdnConfig::default()
        );
    }
}
