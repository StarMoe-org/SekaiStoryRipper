//! Live2D bundle rules of the CN 6.4.0 client (RE-R01).
//!
//! Source of truth: `docs/reverse/cn-6.4.0/live2d-bundle-resolution.yaml`. The constants below are
//! written by hand and `tests::constants_match_the_reverse_engineering_yaml` fails when they drift.

/// yaml `live2d.model_bundle_template`: `Sekai.AssetBundleNames::GetLive2DModelName @0x37281CC`.
pub const MODEL_BUNDLE_PREFIX: &str = "live2d/model/";
/// yaml `live2d.motion_bundle_template`: `Sekai.AssetBundleNames::GetLive2DMotionName @0x3728254`.
pub const MOTION_BUNDLE_PREFIX: &str = "live2d/motion/";
/// yaml `live2d.motion_bundle_template`: `"{0}_motion_base"` literal at `@0x1698638`.
pub const MOTION_BUNDLE_SUFFIX: &str = "_motion_base";
/// yaml `live2d.motion_bundle_exceptions`: the client has no exception table.
pub const MOTION_BUNDLE_EXCEPTIONS: &[(&str, &str)] = &[];
/// yaml `live2d.category_rules_in_use`: `Live2DBuildModelData.CategoryRules` is empty in every model.
pub const CATEGORY_RULES_IN_USE: bool = false;

/// Where a motion or facial name is looked up, in order (yaml `live2d.motion_lookup_order`,
/// `Live2DModel::RegisterMotion @0x205BD28`). Matching is on `<name>.anim` container paths,
/// which Unity stores lower-cased, so names compare case-insensitively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionSource {
    ModelBundle,
    MotionBundle,
}

pub const MOTION_LOOKUP_ORDER: [MotionSource; 2] =
    [MotionSource::ModelBundle, MotionSource::MotionBundle];

/// What the client does when a name is in neither bundle (yaml `live2d.motion_not_found_behavior`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionNotFound {
    /// Logs an error and keeps playing the previous motion/facial on that layer.
    KeepPrevious,
}

pub const MOTION_NOT_FOUND: MotionNotFound = MotionNotFound::KeepPrevious;

/// Model bundle for a costume: `live2d/model/<CostumeType>` (non-AreaTalk playback).
pub fn model_bundle(costume_type: &str) -> String {
    format!("{MODEL_BUNDLE_PREFIX}{costume_type}")
}

/// Motion bundle for a `character2ds.assetName`: `live2d/motion/<assetName>_motion_base`.
pub fn motion_bundle(asset_name: &str) -> String {
    format!("{MOTION_BUNDLE_PREFIX}{asset_name}{MOTION_BUNDLE_SUFFIX}")
}

/// Model key used at playback when `AppearCharacters[].CostumeType` is empty (yaml
/// `live2d.empty_costume_fallback`, `GetExceptionCostumeKey @0x16E1960`): `"{id:D3}_casual"`,
/// with id 0 taken as 1. Never fires on 6.4.0 data, and no such model bundle exists.
pub fn empty_costume_model_key(character2d_id: i64) -> String {
    let id = if character2d_id == 0 {
        1
    } else {
        character2d_id
    };
    if id < 0 {
        format!("-{:03}_casual", id.unsigned_abs())
    } else {
        format!("{id:03}_casual")
    }
}

/// The download builder treats `Character2dId <= 1` as 1 (`csinc` at `@0x16984B8`).
pub fn download_character2d_id(character2d_id: i64) -> i64 {
    if character2d_id > 1 {
        character2d_id
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Entry {
        key: String,
        value: serde_yaml_ng::Value,
    }

    fn yaml_value(key: &str) -> serde_yaml_ng::Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/reverse/cn-6.4.0/live2d-bundle-resolution.yaml"
        );
        let entries: Vec<Entry> =
            serde_yaml_ng::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        entries
            .into_iter()
            .find(|e| e.key == key)
            .unwrap_or_else(|| panic!("{key} missing from yaml"))
            .value
    }

    #[test]
    fn constants_match_the_reverse_engineering_yaml() {
        assert_eq!(
            yaml_value("live2d.model_bundle_template").as_str(),
            Some(model_bundle("{CostumeType}").as_str())
        );
        assert_eq!(
            yaml_value("live2d.motion_bundle_template").as_str(),
            Some(motion_bundle("{character2ds[Character2dId].assetName}").as_str())
        );
        let exceptions = yaml_value("live2d.motion_bundle_exceptions");
        assert_eq!(
            exceptions.as_mapping().map(|m| m.len()),
            Some(MOTION_BUNDLE_EXCEPTIONS.len())
        );
        assert_eq!(
            yaml_value("live2d.category_rules_in_use").as_bool(),
            Some(CATEGORY_RULES_IN_USE)
        );
        assert_eq!(
            yaml_value("live2d.motion_not_found_behavior").as_str(),
            Some("keep_previous")
        );
        assert_eq!(MOTION_NOT_FOUND, MotionNotFound::KeepPrevious);

        let order: Vec<String> =
            serde_yaml_ng::from_value(yaml_value("live2d.motion_lookup_order")).unwrap();
        let expected: Vec<&str> = MOTION_LOOKUP_ORDER
            .iter()
            .map(|source| match source {
                MotionSource::ModelBundle => "model bundle <name>.anim",
                MotionSource::MotionBundle => "motion bundle <name>.anim",
            })
            .collect();
        assert_eq!(order, expected);
    }

    #[test]
    fn empty_costume_fallback_matches_the_yaml() {
        assert_eq!(
            yaml_value("live2d.empty_costume_fallback").as_str(),
            Some("{Character2dId:D3}_casual")
        );
        assert_eq!(empty_costume_model_key(0), "001_casual");
        assert_eq!(empty_costume_model_key(17), "017_casual");
        assert_eq!(empty_costume_model_key(1086), "1086_casual");
        assert_eq!(empty_costume_model_key(-5), "-005_casual");
    }

    #[test]
    fn download_id_is_clamped_to_one() {
        assert_eq!(download_character2d_id(0), 1);
        assert_eq!(download_character2d_id(1), 1);
        assert_eq!(download_character2d_id(366), 366);
    }
}
