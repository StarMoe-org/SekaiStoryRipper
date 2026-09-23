//! CDN settings. Defaults are the CN 6.4.0 iOS client's values; every field can be overridden.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CdnConfig {
    /// Equivalent CDN hosts, tried in order and rotated on transient failures.
    pub hosts: Vec<String>,
    /// `{app}` path segment, e.g. `6.4.0` (not the build number).
    pub app_version: String,
    /// Fixed CDN asset version `N` (`ios{N}`); `None` reads the `version` file first.
    pub asset_version: Option<u32>,
    pub app_id: String,
    pub channel: String,
    pub platform: String,
    pub user_agent: String,
    /// Sent as `x-unity-version`.
    pub unity_version: String,
    /// Parallel bundle downloads.
    pub concurrency: usize,
    /// Attempts per request after the first one.
    pub retries: u32,
    pub timeout_secs: u64,
}

impl Default for CdnConfig {
    fn default() -> Self {
        Self {
            hosts: ["lf3", "lf6", "lf9", "lf26"]
                .iter()
                .map(|h| format!("https://{h}-mkcncdn-tos.dailygn.com"))
                .collect(),
            app_version: "6.4.0".into(),
            asset_version: None,
            app_id: "gdl_app_5236".into(),
            channel: "cn_online".into(),
            platform: "ios".into(),
            user_agent: "cn/4424 CFNetwork/3896.100.1.1.1 Darwin/27.0.0".into(),
            unity_version: "2022.3.62f3".into(),
            concurrency: 8,
            retries: 4,
            timeout_secs: 600,
        }
    }
}

impl CdnConfig {
    /// `…/Mainland/{app}/Release/{channel}/{platform}/version?{cache_buster}`
    pub fn version_url(&self, host: &str, cache_buster: u64) -> String {
        format!(
            "{host}/obj/rt-game-lf/{}/Mainland/{}/Release/{}/{}/version?{cache_buster}",
            self.app_id, self.app_version, self.channel, self.platform
        )
    }

    /// `…/AssetBundle/{app}/Release/{channel}`, the root of manifests and bundles.
    pub fn bundle_root(&self, host: &str) -> String {
        format!(
            "{host}/obj/sf-game-lf/{}/AssetBundle/{}/Release/{}",
            self.app_id, self.app_version, self.channel
        )
    }

    pub fn manifest_url(&self, host: &str, asset_version: u32) -> String {
        format!(
            "{}/{}{asset_version}/AssetBundleInfoNew.json",
            self.bundle_root(host),
            self.platform
        )
    }

    pub fn bundle_url(&self, host: &str, download_path: &str, bundle_name: &str) -> String {
        format!("{}/{download_path}/{bundle_name}", self.bundle_root(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_the_download_guide() {
        let config = CdnConfig::default();
        let host = &config.hosts[0];
        assert_eq!(
            config.version_url(host, 1),
            "https://lf3-mkcncdn-tos.dailygn.com/obj/rt-game-lf/gdl_app_5236/Mainland/6.4.0/Release/cn_online/ios/version?1"
        );
        assert_eq!(
            config.manifest_url(host, 10),
            "https://lf3-mkcncdn-tos.dailygn.com/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios10/AssetBundleInfoNew.json"
        );
        assert_eq!(
            config.bundle_url(host, "ios1", "music/music_score/0074_01"),
            "https://lf3-mkcncdn-tos.dailygn.com/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios1/music/music_score/0074_01"
        );
    }
}
