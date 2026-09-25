//! CDN settings. `CdnConfig::preset` gives each server's iOS client values; every field can be
//! overridden.

use serde::{Deserialize, Serialize};

/// Which game server the assets come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    /// CN (初音未来：缤纷舞台): anonymous CDN.
    #[default]
    Cn,
    /// JP (プロセカ): CloudFront signed cookies issued to a logged-in (guest) account.
    Jp,
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cn => "cn",
            Self::Jp => "jp",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CdnConfig {
    pub region: Region,
    /// CN: equivalent CDN hosts, tried in order and rotated on transient failures.
    /// JP: unused (the bundle host is derived from the version API, see `jp.bundle_host`).
    pub hosts: Vec<String>,
    /// CN: the `{app}` path segment, e.g. `6.4.0` (not the build number). JP: the app version sent
    /// to the version and game APIs, e.g. `6.8.1`.
    pub app_version: String,
    /// Pinned asset version: CN `N` (`ios{N}`), JP e.g. `6.8.0.50` (then also set `jp.asset_hash`).
    /// `None` asks the server (CN: the `version` file; JP: the guest login response).
    pub asset_version: Option<String>,
    /// CN only.
    pub app_id: String,
    /// CN only.
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
    /// JP only.
    pub jp: JpConfig,
}

/// JP 6.8.1 endpoints and client identity (`Sekai.EnvironmentConfig`, `production_ios`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JpConfig {
    /// `VERSION_API_URL_BASE_FORMAT`; `{app}` and `{app_hash}` are substituted. The response
    /// names the API domain, the profile and the asset host hash.
    pub version_api: String,
    /// `x-app-hash`: the build's app hash (`EnvironmentConfig.production_ios` in resources.assets).
    pub app_hash: String,
    /// `ASSETBUNDLE_URL_BASE`; `{profile}` and `{host_hash}` come from the version API.
    pub bundle_host: String,
    /// `ASSETBUNDLE_INFO_URL_BASE`, same placeholders.
    pub info_host: String,
    /// `SIGNED_COOKIE_URL_BASE` + `api/signature` (`PostSignedCookieAuthAPI`).
    pub signed_cookie_url: String,
    /// Pinned asset hash, used with a pinned `asset_version`.
    pub asset_hash: Option<String>,
    pub device_model: String,
    pub operating_system: String,
}

impl Default for JpConfig {
    fn default() -> Self {
        Self {
            version_api: "https://game-version.sekai.colorfulpalette.org/{app}/{app_hash}".into(),
            app_hash: "20dcf972-c5be-4cb6-87af-2185db08a10a".into(),
            bundle_host: "https://{profile}-{host_hash}-assetbundle.sekai.colorfulpalette.org"
                .into(),
            info_host:
                "https://{profile}-{host_hash}-assetbundle-info.sekai.colorfulpalette.org/api/"
                    .into(),
            signed_cookie_url: "https://issue.sekai.colorfulpalette.org/api/signature".into(),
            asset_hash: None,
            device_model: "iPad8,6".into(),
            operating_system: "iPadOS 18.6".into(),
        }
    }
}

impl Default for CdnConfig {
    fn default() -> Self {
        Self::preset(Region::Cn)
    }
}

impl CdnConfig {
    /// The iOS client's values for `region`: CN 6.4.0 (build 4424) or JP 6.8.1 (build 266).
    pub fn preset(region: Region) -> Self {
        let cn = Self {
            region,
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
            jp: JpConfig::default(),
        };
        match region {
            Region::Cn => cn,
            Region::Jp => Self {
                hosts: Vec::new(),
                app_version: "6.8.1".into(),
                app_id: String::new(),
                channel: String::new(),
                user_agent: "ProductName/266 CFNetwork/3826.600.41 Darwin/24.6.0".into(),
                unity_version: "2022.3.62f2".into(),
                ..cn
            },
        }
    }

    /// `…/Mainland/{app}/Release/{channel}/{platform}/version?{cache_buster}` (CN).
    pub fn version_url(&self, host: &str, cache_buster: u64) -> String {
        format!(
            "{host}/obj/rt-game-lf/{}/Mainland/{}/Release/{}/{}/version?{cache_buster}",
            self.app_id, self.app_version, self.channel, self.platform
        )
    }

    /// The root of manifests and bundles: CN `…/AssetBundle/{app}/Release/{channel}`; JP the host.
    pub fn bundle_root(&self, host: &str) -> String {
        match self.region {
            Region::Cn => format!(
                "{host}/obj/sf-game-lf/{}/AssetBundle/{}/Release/{}",
                self.app_id, self.app_version, self.channel
            ),
            Region::Jp => host.trim_end_matches('/').to_owned(),
        }
    }

    /// CN manifest: `…/{platform}{N}/AssetBundleInfoNew.json`.
    pub fn manifest_url(&self, host: &str, asset_version: &str) -> String {
        format!(
            "{}/{}{asset_version}/AssetBundleInfoNew.json",
            self.bundle_root(host),
            self.platform
        )
    }

    /// JP manifest: `{info}version/{assetVersion}/{assetHash}/os/{platform}` (the
    /// `"{0}version/{1}/{2}/os/{3}"` literal; the hash-less form is refused by the CDN).
    pub fn jp_manifest_url(
        &self,
        info_host: &str,
        asset_version: &str,
        asset_hash: &str,
    ) -> String {
        format!(
            "{info_host}version/{asset_version}/{asset_hash}/os/{}",
            self.platform
        )
    }

    /// JP bundles live under `{assetVersion}/{assetHash}/{platform}/`; this is the `downloadPath`
    /// the JP manifest leaves out.
    pub fn jp_download_path(&self, asset_version: &str, asset_hash: &str) -> String {
        format!("{asset_version}/{asset_hash}/{}", self.platform)
    }

    pub fn bundle_url(&self, host: &str, download_path: &str, bundle_name: &str) -> String {
        format!("{}/{download_path}/{bundle_name}", self.bundle_root(host))
    }
}

/// Fills `{name}` placeholders.
pub(crate) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(template.to_owned(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
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
            config.manifest_url(host, "10"),
            "https://lf3-mkcncdn-tos.dailygn.com/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios10/AssetBundleInfoNew.json"
        );
        assert_eq!(
            config.bundle_url(host, "ios1", "music/music_score/0074_01"),
            "https://lf3-mkcncdn-tos.dailygn.com/obj/sf-game-lf/gdl_app_5236/AssetBundle/6.4.0/Release/cn_online/ios1/music/music_score/0074_01"
        );
    }

    #[test]
    fn jp_urls_match_the_client() {
        let config = CdnConfig::preset(Region::Jp);
        let jp = &config.jp;
        let values = [("profile", "production"), ("host_hash", "cf2d2388")];
        let info = fill(&jp.info_host, &values);
        let host = fill(&jp.bundle_host, &values);
        assert_eq!(
            config.jp_manifest_url(&info, "6.8.0.50", "967b24ff"),
            "https://production-cf2d2388-assetbundle-info.sekai.colorfulpalette.org/api/version/6.8.0.50/967b24ff/os/ios"
        );
        let path = config.jp_download_path("6.8.0.50", "967b24ff");
        assert_eq!(
            config.bundle_url(
                &host,
                &path,
                "scenario/unitstory/school-refusal-story-chapter"
            ),
            "https://production-cf2d2388-assetbundle.sekai.colorfulpalette.org/6.8.0.50/967b24ff/ios/scenario/unitstory/school-refusal-story-chapter"
        );
        assert_eq!(
            fill(&jp.version_api, &[("app", "6.8.1"), ("app_hash", "h")]),
            "https://game-version.sekai.colorfulpalette.org/6.8.1/h"
        );
    }
}
