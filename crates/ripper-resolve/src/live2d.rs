//! Live2D bundle rules of the CN 6.4.0 client.

/// `live2d/model/<CostumeType>` (`Sekai.AssetBundleNames::GetLive2DModelName`).
pub const MODEL_BUNDLE_PREFIX: &str = "live2d/model/";
/// `live2d/motion/<assetName>_motion_base` (`Sekai.AssetBundleNames::GetLive2DMotionName`).
pub const MOTION_BUNDLE_PREFIX: &str = "live2d/motion/";
pub const MOTION_BUNDLE_SUFFIX: &str = "_motion_base";
/// The client has no motion-bundle exception table.
pub const MOTION_BUNDLE_EXCEPTIONS: &[(&str, &str)] = &[];
/// `Live2DBuildModelData.CategoryRules` is empty in every model.
pub const CATEGORY_RULES_IN_USE: bool = false;

/// Where a motion or facial name is looked up, in order (`Live2DModel::RegisterMotion`).
/// Matching is on `<name>.anim` container paths, which Unity stores lower-cased, so names
/// compare case-insensitively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionSource {
    ModelBundle,
    MotionBundle,
}

pub const MOTION_LOOKUP_ORDER: [MotionSource; 2] =
    [MotionSource::ModelBundle, MotionSource::MotionBundle];

/// What the client does when a name is in neither bundle.
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

/// Model key used at playback when `AppearCharacters[].CostumeType` is empty
/// (`ScenarioCharacterResourceSet::GetExceptionCostumeKey`): `"{id:D3}_casual"`,
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

/// The download builder treats `Character2dId <= 1` as 1.
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

    #[test]
    fn bundle_names_follow_the_client_templates() {
        assert_eq!(
            model_bundle("18mafuyu_black"),
            "live2d/model/18mafuyu_black"
        );
        assert_eq!(
            motion_bundle("01ichika"),
            "live2d/motion/01ichika_motion_base"
        );
        assert!(MOTION_BUNDLE_EXCEPTIONS.is_empty());
        const { assert!(!CATEGORY_RULES_IN_USE) };
        assert_eq!(MOTION_NOT_FOUND, MotionNotFound::KeepPrevious);
        assert_eq!(
            MOTION_LOOKUP_ORDER,
            [MotionSource::ModelBundle, MotionSource::MotionBundle]
        );
    }

    #[test]
    fn empty_costume_fallback() {
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
