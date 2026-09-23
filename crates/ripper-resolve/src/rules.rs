//! Bundle naming rules of the CN 6.4.0 client, one small function or table per rule.
//!
//! Verified against the 6.4.0 manifest and a corpus of 4849 `ScenarioSceneData`. Every name is
//! derived from masterdata (`scenarioId`, `assetbundleName`, ...) and scenario fields, never
//! from the `ScenarioId` field inside a scenario: 54 scenarios carry another episode's id (e.g.
//! the file `event_168_01` says `event_167_01`), 24 of them naming another existing voice bundle.
//! The Live2D rules are in [`crate::live2d`].

// ---- scenarios ---------------------------------------------------------------------------------

/// `scenario/unitstory/<unitStories.chapters[].assetbundleName>`.
pub fn unit_scenario_bundle(chapter_assetbundle_name: &str) -> String {
    format!("scenario/unitstory/{chapter_assetbundle_name}")
}

/// `event_story/<eventStories.assetbundleName>/scenario`.
pub fn event_scenario_bundle(event_assetbundle_name: &str) -> String {
    format!("event_story/{event_assetbundle_name}/scenario")
}

/// `character/member/<cards[cardId].assetbundleName>` (CN `cardEpisodes` rows have no
/// `assetbundleName` of their own).
pub fn card_scenario_bundle(card_assetbundle_name: &str) -> String {
    format!("character/member/{card_assetbundle_name}")
}

pub const TUTORIAL_SCENARIO_BUNDLE: &str = "scenario/tutorial_story";
pub const ACTIONSET_SCENARIO_PREFIX: &str = "scenario/actionset/";

/// Special-story episodes whose scenario is outside `scenario/special/`. (`op_03` exists in
/// no bundle; its candidates all miss and the episode reports `bundle_missing`.)
pub const SPECIAL_SCENARIO_EXCEPTIONS: &[(&str, &str)] = &[
    ("op_02", TUTORIAL_SCENARIO_BUNDLE),
    ("op_02area", "scenario/actionset/group0"),
];

/// Candidate bundles of a special-story episode, in order:
/// [`SPECIAL_SCENARIO_EXCEPTIONS`], `scenario/special/<episode.assetbundleName>`, then
/// `scenario/special/<specialStory.assetbundleName>` (where the `op_*` episodes live: their
/// `story_sp_ts_01_01` is in no manifest). The caller takes the first candidate whose unpacked
/// directory holds `<scenarioId>.json`.
pub fn special_scenario_bundles(
    scenario_id: &str,
    story_assetbundle_name: &str,
    episode_assetbundle_name: Option<&str>,
) -> Vec<String> {
    let mut candidates: Vec<String> = SPECIAL_SCENARIO_EXCEPTIONS
        .iter()
        .filter(|(id, _)| *id == scenario_id)
        .map(|(_, bundle)| (*bundle).to_owned())
        .collect();
    for name in [episode_assetbundle_name, Some(story_assetbundle_name)]
        .into_iter()
        .flatten()
        .filter(|n| !n.is_empty())
    {
        let bundle = format!("scenario/special/{name}");
        if !candidates.contains(&bundle) {
            candidates.push(bundle);
        }
    }
    candidates
}

// ---- voices ------------------------------------------------------------------------------------

/// Where the voice bundle of a scenario lives, decided by the bundle holding the scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFamily {
    /// unit, event, special, profile: `sound/scenario/voice/<scenarioId>`.
    Scenario,
    /// `sound/card_scenario/voice/<scenarioId>`.
    Card,
    /// `sound/actionset/voice/<scenarioId>`.
    ActionSet,
    /// `sound/tutorial_scenario/voice/<scenarioId>`.
    Tutorial,
}

impl VoiceFamily {
    /// The family of a scenario bundle (card scenarios live in `character/member/`).
    pub fn of_scenario_bundle(scenario_bundle: &str) -> Self {
        if scenario_bundle.starts_with("character/member/") {
            Self::Card
        } else if scenario_bundle.starts_with(ACTIONSET_SCENARIO_PREFIX) {
            Self::ActionSet
        } else if scenario_bundle == TUTORIAL_SCENARIO_BUNDLE {
            Self::Tutorial
        } else {
            Self::Scenario
        }
    }

    /// The scenario's own voice bundle: `<family prefix><scenarioId>` (masterdata `scenarioId`).
    pub fn voice_bundle(self, scenario_id: &str) -> String {
        let prefix = match self {
            Self::Scenario => "sound/scenario/voice/",
            Self::Card => "sound/card_scenario/voice/",
            Self::ActionSet => "sound/actionset/voice/",
            Self::Tutorial => "sound/tutorial_scenario/voice/",
        };
        format!("{prefix}{scenario_id}")
    }
}

/// `VoiceId` prefix of shared per-character lines ("part voices").
pub const PART_VOICE_PREFIX: &str = "partvoice_";

pub fn is_part_voice(voice_id: &str) -> bool {
    voice_id.starts_with(PART_VOICE_PREFIX)
}

/// Part-voice bundle of a character, searched after the scenario's own voice bundle. With
/// `key = <character2ds.assetName>_<character2ds.unit>`: `sound/scenario/voice/part_voice_<key>`
/// when the key starts with `v2_` or `clb`, else `sound/scenario/part_voice/<key>`.
///
/// Known misses (unit `none` characters, `_side` asset names, `Character2dId` 0, lines voiced
/// by another character) are covered by a caller-supplied cue index.
pub fn part_voice_bundle(asset_name: &str, unit: &str) -> String {
    let key = format!("{asset_name}_{unit}");
    if key.starts_with("v2_") || key.starts_with("clb") {
        format!("sound/scenario/voice/part_voice_{key}")
    } else {
        format!("sound/scenario/part_voice/{key}")
    }
}

// ---- SE, BGM, backgrounds, movies, effects -----------------------------------------------------

pub const SE_PACK: &str = "sound/scenario/se/se_pack00001";
pub const SE_PACK_B: &str = "sound/scenario/se/se_pack00001_b";
/// Highest `seNNNNN` in [`SE_PACK`]; later numbers are in [`SE_PACK_B`].
pub const SE_PACK_LAST: u32 = 528;
/// Searched for a cue no rule matches. `sound/scenario/se/se` is a stale subset: never used.
pub const FALLBACK_SE_BUNDLES: [&str; 2] = [SE_PACK, SE_PACK_B];

/// Bundles that may hold an SE cue (`SoundData.Se`, matched exactly), in order:
/// - `seNNNNN` → [`SE_PACK`] up to [`SE_PACK_LAST`], else [`SE_PACK_B`];
/// - `seNNNNN_b`, `se_walk_*`, `se_run_*` → [`SE_PACK_B`];
/// - `se_event_<name>_NNN` → the event's `event_story/<ab>/scenario_se` (both readings of
///   `<ab>`: with and without the `event_` of the cue, the manifest keeps the real one).
///
/// Empty when no rule matches.
pub fn se_bundles(cue: &str) -> Vec<String> {
    let digits = |s: &str| s.len() == 5 && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(number) = cue.strip_prefix("se").filter(|n| digits(n)) {
        let number: u32 = number.parse().unwrap_or(u32::MAX);
        let pack = if number <= SE_PACK_LAST {
            SE_PACK
        } else {
            SE_PACK_B
        };
        return vec![pack.to_owned()];
    }
    if cue
        .strip_prefix("se")
        .and_then(|n| n.strip_suffix("_b"))
        .is_some_and(digits)
        || cue.starts_with("se_walk_")
        || cue.starts_with("se_run_")
    {
        return vec![SE_PACK_B.to_owned()];
    }
    if let Some(rest) = cue.strip_prefix("se_event_")
        && let Some((name, number)) = rest.rsplit_once('_')
        && !name.is_empty()
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
    {
        return vec![
            event_se_bundle(&format!("event_{name}")),
            event_se_bundle(name),
        ];
    }
    Vec::new()
}

/// `event_story/<eventStories.assetbundleName>/scenario_se`: an event's own SE cues.
pub fn event_se_bundle(event_assetbundle_name: &str) -> String {
    format!("event_story/{event_assetbundle_name}/scenario_se")
}

/// `sound/scenario/bgm/<FirstBgm | SoundData.Bgm>`.
pub fn bgm_bundle(name: &str) -> String {
    format!("sound/scenario/bgm/{name}")
}

/// `NeedBundleNames` prefix of background bundles.
pub const BACKGROUND_PREFIX: &str = "scenario/background/";
/// `NeedBundleNames` prefix of movie bundles (`NeedBundleNames` = backgrounds ∪ movies).
pub const MOVIE_PREFIX: &str = "scenario/movie/";

/// `scenario/background/<FirstBackground | EffectType 7 StringVal>`.
pub fn background_bundle(name: &str) -> String {
    format!("{BACKGROUND_PREFIX}{name}")
}

/// `scenario/movie/<EffectType 19 StringVal>`.
pub fn movie_bundle(name: &str) -> String {
    format!("{MOVIE_PREFIX}{name}")
}

/// EffectType 22 (`AttachCharacterShader`) `StringVal` → effect bundle. `none` and `monitor`
/// need no bundle; `blur` carries a number in `StringValSub`, not a bundle.
pub const CHARACTER_SHADER_BUNDLES: &[(&str, &str)] = &[("hologram", "scenario/effect/hologram")];

pub fn character_shader_bundle(shader: &str) -> Option<&'static str> {
    CHARACTER_SHADER_BUNDLES
        .iter()
        .find(|(name, _)| *name == shader)
        .map(|(_, bundle)| *bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_candidates_put_exceptions_first() {
        assert_eq!(
            special_scenario_bundles("op_02", "special-story", Some("story_sp_ts_01_01")),
            [
                "scenario/tutorial_story",
                "scenario/special/story_sp_ts_01_01",
                "scenario/special/special-story"
            ]
        );
        assert_eq!(
            special_scenario_bundles("op_02area", "special-story", None)[0],
            "scenario/actionset/group0"
        );
        assert_eq!(
            special_scenario_bundles(
                "1st_countdown_01",
                "special-story",
                Some("story_1st_countdown")
            ),
            [
                "scenario/special/story_1st_countdown",
                "scenario/special/special-story"
            ]
        );
        assert_eq!(
            special_scenario_bundles("x", "", Some("")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn voice_bundles_follow_the_scenario_bundle() {
        for (scenario_bundle, voice) in [
            (
                "scenario/unitstory/idol-story-chapter",
                "sound/scenario/voice/x",
            ),
            (
                "event_story/event_a_2021/scenario",
                "sound/scenario/voice/x",
            ),
            ("scenario/special/special-story", "sound/scenario/voice/x"),
            (
                "character/member/res001_no001",
                "sound/card_scenario/voice/x",
            ),
            ("scenario/actionset/group0", "sound/actionset/voice/x"),
            ("scenario/tutorial_story", "sound/tutorial_scenario/voice/x"),
        ] {
            assert_eq!(
                VoiceFamily::of_scenario_bundle(scenario_bundle).voice_bundle("x"),
                voice
            );
        }
    }

    #[test]
    fn part_voice_bundles() {
        assert!(is_part_voice("partvoice_21miku_01"));
        assert!(!is_part_voice("voice_ms_night10_01_17"));
        assert_eq!(
            part_voice_bundle("21miku", "school_refusal"),
            "sound/scenario/part_voice/21miku_school_refusal"
        );
        assert_eq!(
            part_voice_bundle("v2_21miku", "idol"),
            "sound/scenario/voice/part_voice_v2_21miku_idol"
        );
        assert_eq!(
            part_voice_bundle("clb01_22rin", "piapro"),
            "sound/scenario/voice/part_voice_clb01_22rin_piapro"
        );
    }

    #[test]
    fn se_rules() {
        assert_eq!(se_bundles("se00001"), [SE_PACK]);
        assert_eq!(se_bundles("se00528"), [SE_PACK]);
        assert_eq!(se_bundles("se00529"), [SE_PACK_B]);
        assert_eq!(se_bundles("se00012_b"), [SE_PACK_B]);
        assert_eq!(se_bundles("se_walk_01"), [SE_PACK_B]);
        assert_eq!(se_bundles("se_run_asphalt"), [SE_PACK_B]);
        assert_eq!(
            se_bundles("se_event_stella_2020_001"),
            [
                "event_story/event_stella_2020/scenario_se",
                "event_story/stella_2020/scenario_se"
            ]
        );
        for unmatched in [
            "se0001",
            "se000011",
            "bgm00001",
            "se_event_",
            "se_event_x_y",
            "se00001 ",
        ] {
            assert!(se_bundles(unmatched).is_empty(), "{unmatched}");
        }
    }

    #[test]
    fn other_bundles() {
        assert_eq!(bgm_bundle("bgm00000"), "sound/scenario/bgm/bgm00000");
        assert_eq!(
            background_bundle("bg_a000000"),
            "scenario/background/bg_a000000"
        );
        assert_eq!(movie_bundle("op"), "scenario/movie/op");
        assert_eq!(
            character_shader_bundle("hologram"),
            Some("scenario/effect/hologram")
        );
        assert_eq!(character_shader_bundle("blur"), None);
        assert_eq!(character_shader_bundle("none"), None);
    }
}
