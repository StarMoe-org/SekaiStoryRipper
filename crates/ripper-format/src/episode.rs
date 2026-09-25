//! `ripper-episode` v1: everything one story episode needs, as paths into the library.
//!
//! Written to `episodes/…` next to `library/`. Every `path` in this document is relative to the
//! library root (`library/`) and `/`-separated, so a reader joins it onto the library directory
//! as is; `bundle` names the `library/<bundle>/` directory the file was unpacked into.
//!
//! The index is produced in two phases by `ripper-resolve`: the bundle plan (from masterdata, the
//! scenario and the manifest) and the file lookup (from the unpack records). Anything that could
//! not be resolved is listed in `warnings` instead of failing (ADR-0009).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ripper-episode";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeIndex {
    pub format: String,
    pub version: u32,
    pub story: StoryInfo,
    /// The `ScenarioSceneData` typetree JSON.
    pub scenario: FileRef,
    /// Every bundle the episode needs (including manifest dependencies), sorted and unique.
    pub bundles: Vec<String>,
    /// Keyed by the scenario's `Character2dId`.
    pub characters: BTreeMap<i64, Character>,
    /// Keyed by background name (`FirstBackground`, EffectType 7 `StringVal`, or the
    /// `scenario/background/<name>` of `NeedBundleNames`). When EffectType 7 names a file other
    /// than the bundle's own (`StringValSub`), the key is `<name>/<file>`.
    pub backgrounds: BTreeMap<String, FileRef>,
    /// Keyed by BGM name (`FirstBgm`, `SoundData.Bgm`).
    pub bgm: BTreeMap<String, AudioRef>,
    /// Keyed by SE cue name (`SoundData.Se`).
    pub se: BTreeMap<String, AudioRef>,
    /// Keyed by `VoiceId` (`TalkData[].Voices[]`, EffectType 24 `StringValSub`).
    pub voices: BTreeMap<String, AudioRef>,
    /// Effect name (EffectType 15/16/22 `StringVal`, or the last segment of an
    /// `IncludeSoundDataBundleNames` entry) → bundle name.
    pub effects: BTreeMap<String, String>,
    /// Keyed by movie name (EffectType 19 `StringVal`, `scenario/movie/<name>`).
    pub movies: BTreeMap<String, MovieRef>,
    /// Music video ids (`EpisodeMusicVideoId`, EffectType 37 `IntVal`). Out of scope for v1:
    /// listed so a player can show a placeholder.
    pub music_videos: Vec<String>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryType {
    /// `unitStories`: main story chapters.
    Unit,
    /// `eventStories`.
    Event,
    /// `cardEpisodes`.
    Card,
    /// `specialStories`.
    Special,
}

impl StoryType {
    /// The selector prefix and `episodes/<type>/` directory name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Event => "event",
            Self::Card => "card",
            Self::Special => "special",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryInfo {
    #[serde(rename = "type")]
    pub story_type: StoryType,
    /// Canonical selector of this episode, e.g. `unit:school-refusal-story-chapter/3`.
    pub selector: String,
    /// unit: chapter id; event: `eventId`; card: `cardId`; special: `specialStories.id`.
    pub story_id: i64,
    /// unit/event/special: `episodeNo`; card: 1 for the first part, 2 for the second.
    pub episode_no: i64,
    /// Id of the masterdata episode row.
    pub episode_id: i64,
    pub title: String,
    pub scenario_id: String,
}

/// A file of the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub bundle: String,
    /// Relative to `library/`, `/`-separated; starts with `<bundle>/`.
    pub path: String,
}

impl FileRef {
    /// A file at `relative` (relative to `library/<bundle>/`, as in the unpack record).
    pub fn in_bundle(bundle: &str, relative: &str) -> Self {
        Self {
            bundle: bundle.to_owned(),
            path: library_path(bundle, relative),
        }
    }
}

/// `library/`-relative path of a file stored at `relative` inside `library/<bundle>/`.
pub fn library_path(bundle: &str, relative: &str) -> String {
    format!("{bundle}/{relative}")
}

/// One Live2D character (`Character2dId`) of the episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    /// `character2ds[id].assetName`; `None` when the id has no row or no asset name.
    pub asset_name: Option<String>,
    /// `live2d/motion/<assetName>_motion_base`, shared by every costume; `None` when it could not
    /// be resolved or is not in the manifest (see `warnings`).
    pub motion_bundle: Option<String>,
    /// In first-appearance order; the first one is the costume the character starts in.
    pub costumes: Vec<Costume>,
}

/// One `(Character2dId, CostumeType)` pair: the game's unit of Live2D resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Costume {
    pub costume_type: String,
    /// `live2d/model/<CostumeType>`; `None` when it is not in the manifest.
    pub model_bundle: Option<String>,
    /// The motion and facial names the episode uses for this character, resolved as the game does
    /// with this costume's model loaded: `<name>.anim` in the model bundle first, then in the
    /// motion bundle, case-insensitively (a clip `x.anim` is stored as `x.sse-motion.json`).
    /// Keys are the names as written in the scenario. A name missing here makes the game keep
    /// the previous motion/facial on that layer.
    pub motions: BTreeMap<String, FileRef>,
}

/// One cue of an unpacked ACB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRef {
    pub bundle: String,
    /// The raw `.acb`, relative to `library/`.
    pub acb: String,
    pub cue: String,
    /// The cue's decoded waveforms in track order, relative to `library/`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieRef {
    pub bundle: String,
    /// Every file unpacked from the movie bundle, relative to `library/`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub kind: WarningKind,
    pub detail: String,
}

impl Warning {
    pub fn new(kind: WarningKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// The motion bundle of a `Character2dId` is unknown: no `assetName`, or
    /// `live2d/motion/<assetName>_motion_base` is not in the manifest. The game plays no motion.
    UnresolvedMotionBundle,
    /// `live2d/model/<CostumeType>` is not in the manifest (e.g. the game's own typos `"3"`, `"\\"`).
    ModelBundleMissing,
    /// A motion/facial name is in none of the character's model bundles nor its motion bundle.
    MotionNotFound,
    /// The scenario uses a `Character2dId` that `character2ds` does not have: masterdata and
    /// manifest (or scenario) are from different versions, not a rule failure.
    MasterManifestMismatch,
    /// A bundle the episode references is not in the manifest.
    BundleMissing,
    /// A planned bundle has no unpack record.
    BundleNotUnpacked,
    /// A file expected inside an unpacked bundle is not in its record (or was skipped).
    FileNotFound,
    /// No searched ACB has the cue.
    CueNotFound,
    /// An `EffectType` the 6.4.0 client does not know (it skips the snippet).
    UnknownEffectType,
    /// A motion/facial is given for a `Character2dId` that is not in `AppearCharacters`.
    UnknownCharacter,
    /// Something the episode uses that v1 does not export (music videos).
    OutOfScope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EpisodeIndex {
        let motion = FileRef::in_bundle(
            "live2d/motion/17kanade_motion_base",
            "motion/w-kanade-angry01.sse-motion.json",
        );
        EpisodeIndex {
            format: FORMAT.into(),
            version: VERSION,
            story: StoryInfo {
                story_type: StoryType::Unit,
                selector: "unit:school-refusal-story-chapter/2".into(),
                story_id: 5,
                episode_no: 2,
                episode_id: 50001,
                title: "t".into(),
                scenario_id: "nightcode_01_01".into(),
            },
            scenario: FileRef::in_bundle(
                "scenario/unitstory/school-refusal-story-chapter",
                "nightcode_01_01.json",
            ),
            bundles: vec!["a".into()],
            characters: BTreeMap::from([(
                17,
                Character {
                    asset_name: Some("17kanade".into()),
                    motion_bundle: Some("live2d/motion/17kanade_motion_base".into()),
                    costumes: vec![Costume {
                        costume_type: "17kanade_normal".into(),
                        model_bundle: Some("live2d/model/17kanade_normal".into()),
                        motions: BTreeMap::from([("w-kanade-angry01".into(), motion)]),
                    }],
                },
            )]),
            backgrounds: BTreeMap::new(),
            bgm: BTreeMap::from([(
                "bgm00000".into(),
                AudioRef {
                    bundle: "sound/scenario/bgm/bgm00000".into(),
                    acb: "sound/scenario/bgm/bgm00000/bgm00000.acb".into(),
                    cue: "bgm00000".into(),
                    files: vec!["sound/scenario/bgm/bgm00000/bgm00000.audio/e0.wav".into()],
                },
            )]),
            se: BTreeMap::new(),
            voices: BTreeMap::new(),
            effects: BTreeMap::from([("hologram".into(), "scenario/effect/hologram".into())]),
            movies: BTreeMap::new(),
            music_videos: vec!["5".into()],
            warnings: vec![Warning::new(WarningKind::OutOfScope, "music video 5")],
        }
    }

    #[test]
    fn round_trips_with_camel_case_fields_and_snake_case_kinds() {
        let index = sample();
        let json = serde_json::to_value(&index).unwrap();
        assert_eq!(json["format"], "ripper-episode");
        assert_eq!(json["story"]["type"], "unit");
        assert_eq!(json["story"]["scenarioId"], "nightcode_01_01");
        assert_eq!(json["musicVideos"][0], "5");
        assert_eq!(json["warnings"][0]["kind"], "out_of_scope");
        let character = &json["characters"]["17"];
        assert_eq!(
            character["motionBundle"],
            "live2d/motion/17kanade_motion_base"
        );
        assert_eq!(
            character["costumes"][0]["motions"]["w-kanade-angry01"]["path"],
            "live2d/motion/17kanade_motion_base/motion/w-kanade-angry01.sse-motion.json"
        );
        let back: EpisodeIndex = serde_json::from_value(json).unwrap();
        assert_eq!(back, index);
    }

    #[test]
    fn warning_kinds_serialise_as_documented() {
        for (kind, name) in [
            (
                WarningKind::UnresolvedMotionBundle,
                "unresolved_motion_bundle",
            ),
            (WarningKind::ModelBundleMissing, "model_bundle_missing"),
            (WarningKind::MotionNotFound, "motion_not_found"),
            (
                WarningKind::MasterManifestMismatch,
                "master_manifest_mismatch",
            ),
            (WarningKind::BundleMissing, "bundle_missing"),
            (WarningKind::CueNotFound, "cue_not_found"),
            (WarningKind::UnknownEffectType, "unknown_effect_type"),
            (WarningKind::OutOfScope, "out_of_scope"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
    }

    #[test]
    fn library_paths_are_bundle_prefixed() {
        let file = FileRef::in_bundle("scenario/background/bg_a000000", "bg_a000000.png");
        assert_eq!(file.path, "scenario/background/bg_a000000/bg_a000000.png");
    }
}
