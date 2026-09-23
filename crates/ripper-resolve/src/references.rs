//! What one `ScenarioSceneData` (typetree JSON, C# field names) refers to.
//!
//! Arrays are walked in index order (`Snippets[].Index` is not an order key), and every list
//! keeps first-seen order without duplicates, so the result is stable for a given scenario.
//! Missing fields read as empty: the extractor never fails on a well-formed object.

use std::collections::HashSet;
use std::hash::Hash;

use ripper_format::episode::{Warning, WarningKind};
use serde_json::Value;

use crate::live2d;
use crate::rules::{BACKGROUND_PREFIX, MOVIE_PREFIX, character_shader_bundle};

/// Highest `SpecialEffectType` the 6.4.0 client handles (its jump table covers 1..=44).
pub const MAX_KNOWN_EFFECT_TYPE: i64 = 44;

/// `ScenarioSnippetSpecialEffect.SpecialEffectType` values the resolver reads.
pub mod effect_type {
    pub const CHANGE_BACKGROUND: i64 = 7;
    pub const PLAY_SCENARIO_EFFECT: i64 = 15;
    pub const STOP_SCENARIO_EFFECT: i64 = 16;
    pub const MOVIE: i64 = 19;
    pub const ATTACH_CHARACTER_SHADER: i64 = 22;
    pub const FULL_SCREEN_TEXT: i64 = 24;
    pub const MUSIC_VIDEO: i64 = 37;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    #[error("scenario JSON is not an object")]
    NotAnObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScenarioRefs {
    /// The `ScenarioId` field, for information only: it can name another episode (54 of 4849
    /// scenarios), so bundle names always come from the masterdata `scenarioId`.
    pub scenario_id: String,
    /// `AppearCharacters` order; one entry per `Character2dId`.
    pub characters: Vec<CharacterRefs>,
    pub backgrounds: Vec<BackgroundRef>,
    /// Movie names (`scenario/movie/<name>`).
    pub movies: Vec<String>,
    pub voices: Vec<VoiceRef>,
    pub bgm: Vec<String>,
    pub se: Vec<SeRef>,
    pub effects: Vec<EffectRef>,
    pub music_videos: Vec<String>,
    /// Unknown effect types and motions of characters that never appear.
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRefs {
    /// `Character2dId` as written in the scenario (0 included: the download builder maps it to 1).
    pub id: i64,
    /// First-appearance order; the first is the costume the character is loaded in. An empty
    /// `AppearCharacters` costume becomes [`live2d::empty_costume_model_key`].
    pub costumes: Vec<String>,
    /// Motion and facial names (FirstLayout, LayoutData, TalkData[].Motions[]), as written.
    pub motions: Vec<String>,
}

/// A background texture: `scenario/background/<name>`, file `<file>` inside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackgroundRef {
    pub name: String,
    pub file: String,
}

impl BackgroundRef {
    /// Key of the background in the episode index: the name, or `<name>/<file>` when they differ.
    pub fn key(&self) -> String {
        if self.file == self.name {
            self.name.clone()
        } else {
            format!("{}/{}", self.name, self.file)
        }
    }
}

/// A `VoiceId` (matched exactly, trailing spaces included) and who says it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoiceRef {
    pub id: String,
    /// `TalkData[].Voices[].Character2dId`; 0 for EffectType 24 (`FullScreenText`).
    pub character2d_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SeRef {
    pub cue: String,
    /// `SoundData.SeBundleName` when set: searched before the configured SE bundles.
    pub bundle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectRef {
    pub name: String,
    pub bundle: String,
}

/// Order-preserving set.
struct Unique<T> {
    items: Vec<T>,
    seen: HashSet<T>,
}

impl<T: Clone + Eq + Hash> Unique<T> {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn push(&mut self, item: T) {
        if self.seen.insert(item.clone()) {
            self.items.push(item);
        }
    }
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn int(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[derive(Default)]
struct CharacterBuilder {
    order: Vec<i64>,
    costumes: Vec<Unique<String>>,
    motions: Vec<Unique<String>>,
    appeared: usize,
}

impl CharacterBuilder {
    fn slot(&mut self, id: i64) -> usize {
        match self.order.iter().position(|&i| i == id) {
            Some(slot) => slot,
            None => {
                self.order.push(id);
                self.costumes.push(Unique::new());
                self.motions.push(Unique::new());
                self.order.len() - 1
            }
        }
    }

    fn appeared(&self, id: i64) -> bool {
        self.order[..self.appeared].contains(&id)
    }

    fn costume(&mut self, id: i64, costume: &str) {
        if !costume.is_empty() {
            let slot = self.slot(id);
            self.costumes[slot].push(costume.to_owned());
        }
    }

    fn names(&mut self, id: i64, names: [&str; 2], warnings: &mut Vec<Warning>, context: &str) {
        let names: Vec<&str> = names.into_iter().filter(|n| !n.is_empty()).collect();
        if names.is_empty() {
            return;
        }
        if !self.appeared(id) {
            warnings.push(Warning::new(
                WarningKind::UnknownCharacter,
                format!("{context}: Character2dId {id} is not in AppearCharacters ({names:?})"),
            ));
            return;
        }
        let slot = self.slot(id);
        for name in names {
            self.motions[slot].push(name.to_owned());
        }
    }
}

/// Extracts every asset reference of one scenario.
pub fn extract(scenario: &Value) -> Result<ScenarioRefs, ReferenceError> {
    if !scenario.is_object() {
        return Err(ReferenceError::NotAnObject);
    }
    let scenario_id = string(scenario, "ScenarioId");
    let mut warnings = Vec::new();

    // Characters: AppearCharacters decides who is loaded (live2d-bundle-resolution.md §1.1).
    let mut characters = CharacterBuilder::default();
    for set in array(scenario, "AppearCharacters") {
        let id = int(set, "Character2dId");
        let costume = string(set, "CostumeType");
        let slot = characters.slot(id);
        characters.appeared = characters.appeared.max(slot + 1);
        if costume.is_empty() {
            characters.costumes[slot].push(live2d::empty_costume_model_key(id));
        } else {
            characters.costume(id, costume);
        }
    }
    for (i, layout) in array(scenario, "FirstLayout").iter().enumerate() {
        let id = int(layout, "Character2dId");
        if characters.appeared(id) {
            characters.costume(id, string(layout, "CostumeType"));
        }
        let names = [string(layout, "MotionName"), string(layout, "FacialName")];
        characters.names(id, names, &mut warnings, &format!("FirstLayout[{i}]"));
    }
    for (i, layout) in array(scenario, "LayoutData").iter().enumerate() {
        let id = int(layout, "Character2dId");
        if characters.appeared(id) {
            characters.costume(id, string(layout, "CostumeType"));
        }
        let names = [string(layout, "MotionName"), string(layout, "FacialName")];
        characters.names(id, names, &mut warnings, &format!("LayoutData[{i}]"));
    }

    let mut voices = Unique::new();
    for (i, talk) in array(scenario, "TalkData").iter().enumerate() {
        for (j, motion) in array(talk, "Motions").iter().enumerate() {
            let id = int(motion, "Character2dId");
            let names = [string(motion, "MotionName"), string(motion, "FacialName")];
            let context = format!("TalkData[{i}].Motions[{j}]");
            characters.names(id, names, &mut warnings, &context);
        }
        for voice in array(talk, "Voices") {
            let id = string(voice, "VoiceId");
            if !id.is_empty() {
                voices.push(VoiceRef {
                    id: id.to_owned(),
                    character2d_id: int(voice, "Character2dId"),
                });
            }
        }
    }

    let mut backgrounds = Unique::new();
    let first_background = string(scenario, "FirstBackground");
    if !first_background.is_empty() {
        backgrounds.push(BackgroundRef {
            name: first_background.into(),
            file: first_background.into(),
        });
    }
    let mut movies = Unique::new();
    for name in array(scenario, "NeedBundleNames")
        .iter()
        .filter_map(Value::as_str)
    {
        if let Some(bg) = name
            .strip_prefix(BACKGROUND_PREFIX)
            .filter(|n| !n.is_empty())
        {
            backgrounds.push(BackgroundRef {
                name: bg.into(),
                file: bg.into(),
            });
        } else if let Some(movie) = name.strip_prefix(MOVIE_PREFIX).filter(|n| !n.is_empty()) {
            movies.push(movie.to_owned());
        }
    }

    let mut effects = Unique::new();
    let mut music_videos = Unique::new();
    let mv = string(scenario, "EpisodeMusicVideoId");
    if !mv.is_empty() {
        music_videos.push(mv.to_owned());
    }
    for (i, effect) in array(scenario, "SpecialEffectData").iter().enumerate() {
        let effect_type = int(effect, "EffectType");
        let val = string(effect, "StringVal");
        let sub = string(effect, "StringValSub");
        match effect_type {
            effect_type::CHANGE_BACKGROUND if !val.is_empty() => {
                backgrounds.push(BackgroundRef {
                    name: val.into(),
                    file: if sub.is_empty() { val } else { sub }.into(),
                });
            }
            effect_type::MOVIE if !val.is_empty() => movies.push(val.to_owned()),
            effect_type::FULL_SCREEN_TEXT if !sub.is_empty() => voices.push(VoiceRef {
                id: sub.to_owned(),
                character2d_id: 0,
            }),
            effect_type::MUSIC_VIDEO => music_videos.push(int(effect, "IntVal").to_string()),
            // `StringVal` names the effect, `StringValSub` is its bundle.
            effect_type::PLAY_SCENARIO_EFFECT | effect_type::STOP_SCENARIO_EFFECT
                if !sub.is_empty() =>
            {
                let name = if val.is_empty() {
                    last_segment(sub)
                } else {
                    val
                };
                effects.push(EffectRef {
                    name: name.to_owned(),
                    bundle: sub.to_owned(),
                });
            }
            effect_type::PLAY_SCENARIO_EFFECT => warnings.push(Warning::new(
                WarningKind::BundleMissing,
                format!("SpecialEffectData[{i}]: EffectType 15 {val:?} names no bundle"),
            )),
            // `hologram` has a bundle; `none`/`monitor` need none and `blur` carries a number.
            effect_type::ATTACH_CHARACTER_SHADER => {
                if let Some(bundle) = character_shader_bundle(val) {
                    effects.push(EffectRef {
                        name: val.to_owned(),
                        bundle: bundle.to_owned(),
                    });
                }
            }
            t if !(0..=MAX_KNOWN_EFFECT_TYPE).contains(&t) => warnings.push(Warning::new(
                WarningKind::UnknownEffectType,
                format!(
                    "SpecialEffectData[{i}]: EffectType {t} is newer than the 6.4.0 client \
                     (skipped by the game; StringVal {val:?}, StringValSub {sub:?})"
                ),
            )),
            _ => {}
        }
    }
    for bundle in array(scenario, "IncludeSoundDataBundleNames")
        .iter()
        .filter_map(Value::as_str)
        .filter(|b| !b.is_empty())
    {
        effects.push(EffectRef {
            name: last_segment(bundle).to_owned(),
            bundle: bundle.to_owned(),
        });
    }

    let mut bgm = Unique::new();
    let first_bgm = string(scenario, "FirstBgm");
    if !first_bgm.is_empty() {
        bgm.push(first_bgm.to_owned());
    }
    let mut se = Unique::new();
    for sound in array(scenario, "SoundData") {
        let name = string(sound, "Bgm");
        if !name.is_empty() {
            bgm.push(name.to_owned());
        }
        let cue = string(sound, "Se");
        if !cue.is_empty() {
            let bundle = string(sound, "SeBundleName");
            se.push(SeRef {
                cue: cue.to_owned(),
                bundle: (!bundle.is_empty()).then(|| bundle.to_owned()),
            });
        }
    }

    let CharacterBuilder {
        order,
        costumes,
        motions,
        appeared,
    } = characters;
    let characters = order
        .into_iter()
        .zip(costumes)
        .zip(motions)
        .take(appeared)
        .map(|((id, costumes), motions)| CharacterRefs {
            id,
            costumes: costumes.items,
            motions: motions.items,
        })
        .collect();

    Ok(ScenarioRefs {
        scenario_id: scenario_id.to_owned(),
        characters,
        backgrounds: backgrounds.items,
        movies: movies.items,
        voices: voices.items,
        bgm: bgm.items,
        se: se.items,
        effects: effects.items,
        music_videos: music_videos.items,
        warnings,
    })
}

fn last_segment(bundle: &str) -> &str {
    bundle.rsplit('/').next().unwrap_or(bundle)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A synthetic scenario touching every reference kind.
    pub(crate) fn scenario() -> Value {
        json!({
            "m_Name": "test_01_01",
            "ScenarioId": "test_01_01",
            "AppearCharacters": [
                {"Character2dId": 17, "CostumeType": "17kanade_normal"},
                {"Character2dId": 18, "CostumeType": "18mafuyu_normal"},
                {"Character2dId": 17, "CostumeType": "17kanade_cloth001"},
                {"Character2dId": 5, "CostumeType": ""}
            ],
            "FirstLayout": [
                {"PositionSide": 3, "Character2dId": 17, "CostumeType": "", "MotionName": "w-kanade-idle01",
                 "FacialName": "face_normal_01", "OffsetX": 0}
            ],
            "FirstBgm": "bgm00000",
            "EpisodeMusicVideoId": "",
            "FirstBackground": "bg_a000000",
            "Snippets": [],
            "TalkData": [
                {"TalkCharacters": [{"Character2dId": 0}], "Body": "…", "Motions": [], "Voices": []},
                {"TalkCharacters": [{"Character2dId": 18}], "Body": "…",
                 "Motions": [
                     {"Character2dId": 18, "MotionName": "w-mafuyu-Nod01", "FacialName": "", "TimingSyncValue": 0},
                     {"Character2dId": 99, "MotionName": "w-ghost01", "FacialName": "", "TimingSyncValue": 0}
                 ],
                 "Voices": [{"Character2dId": 18, "VoiceId": "voice_18_01", "Volume": 1}]},
                {"TalkCharacters": [{"Character2dId": 18}], "Body": "…", "Motions": [],
                 "Voices": [{"Character2dId": 18, "VoiceId": "voice_18_01", "Volume": 1}]},
                {"TalkCharacters": [{"Character2dId": 17}], "Body": "…", "Motions": [],
                 "Voices": [{"Character2dId": 17, "VoiceId": "partvoice_17kanade_01", "Volume": 1}]}
            ],
            "LayoutData": [
                {"Type": 2, "Character2dId": 17, "CostumeType": "17kanade_black", "MotionName": "w-kanade-angry01",
                 "FacialName": "face_angry_01"},
                {"Type": 0, "Character2dId": 18, "CostumeType": "", "MotionName": "", "FacialName": "face_smile_01"}
            ],
            "SpecialEffectData": [
                {"EffectType": 7, "StringVal": "bg_e000101", "StringValSub": "bg_e000101", "Duration": 1, "IntVal": 0},
                {"EffectType": 7, "StringVal": "bg_e000102", "StringValSub": "bg_e000102_night", "Duration": 1, "IntVal": 0},
                {"EffectType": 8, "StringVal": "Telop", "StringValSub": "", "Duration": 0, "IntVal": 0},
                {"EffectType": 15, "StringVal": "fx_rain", "StringValSub": "scenario/effect/rain", "Duration": 0, "IntVal": 0},
                {"EffectType": 16, "StringVal": "fx_rain", "StringValSub": "", "Duration": 0, "IntVal": 0},
                {"EffectType": 19, "StringVal": "school_refusal_opening", "StringValSub": "", "Duration": 0, "IntVal": 0},
                {"EffectType": 22, "StringVal": "hologram", "StringValSub": "scenario/effect/hologram", "Duration": 0, "IntVal": 17},
                {"EffectType": 22, "StringVal": "none", "StringValSub": "", "Duration": 0, "IntVal": 17},
                {"EffectType": 22, "StringVal": "blur", "StringValSub": "1.75", "Duration": 0, "IntVal": 17},
                {"EffectType": 15, "StringVal": "fx_nothing", "StringValSub": "", "Duration": 0, "IntVal": 0},
                {"EffectType": 24, "StringVal": "full screen", "StringValSub": "voice_op_01", "Duration": 0, "IntVal": 0},
                {"EffectType": 37, "StringVal": "", "StringValSub": "", "Duration": 0, "IntVal": 5},
                {"EffectType": 45, "StringVal": "Zoom：1.2", "StringValSub": "Linear", "Duration": 25, "IntVal": 0}
            ],
            "SoundData": [
                {"PlayMode": 0, "Bgm": "", "Se": "se00001", "Volume": 1, "SeBundleName": "", "Duration": 0},
                {"PlayMode": 0, "Bgm": "bgm00021", "Se": "", "Volume": 1, "SeBundleName": "", "Duration": 0},
                {"PlayMode": 0, "Bgm": "", "Se": "se_ev01", "Volume": 1,
                 "SeBundleName": "event_story/event_x_2023/scenario_se", "Duration": 0},
                {"PlayMode": 0, "Bgm": "bgm00000", "Se": "se00001", "Volume": 1, "SeBundleName": "", "Duration": 0},
                {"PlayMode": 0, "Bgm": "", "Se": "se00600", "Volume": 1, "SeBundleName": "", "Duration": 0},
                {"PlayMode": 0, "Bgm": "", "Se": "se_event_x_2023_001", "Volume": 1, "SeBundleName": "", "Duration": 0}
            ],
            "NeedBundleNames": [
                "scenario/background/bg_a000000",
                "scenario/background/bg_h000104",
                "scenario/movie/school_refusal_opening",
                "scenario/something/else"
            ],
            "IncludeSoundDataBundleNames": ["scenario/effect/hologram"]
        })
    }

    #[test]
    fn extracts_characters_costumes_and_motions() {
        let refs = extract(&scenario()).unwrap();
        assert_eq!(refs.scenario_id, "test_01_01");
        let ids: Vec<i64> = refs.characters.iter().map(|c| c.id).collect();
        assert_eq!(ids, [17, 18, 5]);
        let kanade = &refs.characters[0];
        assert_eq!(
            kanade.costumes,
            ["17kanade_normal", "17kanade_cloth001", "17kanade_black"]
        );
        assert_eq!(
            kanade.motions,
            [
                "w-kanade-idle01",
                "face_normal_01",
                "w-kanade-angry01",
                "face_angry_01"
            ]
        );
        let mafuyu = &refs.characters[1];
        assert_eq!(mafuyu.costumes, ["18mafuyu_normal"]);
        assert_eq!(mafuyu.motions, ["face_smile_01", "w-mafuyu-Nod01"]);
        let empty_costume = &refs.characters[2];
        assert_eq!(empty_costume.costumes, ["005_casual"]);
        assert!(empty_costume.motions.is_empty());
    }

    #[test]
    fn extracts_backgrounds_movies_audio_effects_and_mvs() {
        let refs = extract(&scenario()).unwrap();
        let backgrounds: Vec<String> = refs.backgrounds.iter().map(BackgroundRef::key).collect();
        assert_eq!(
            backgrounds,
            [
                "bg_a000000",
                "bg_h000104",
                "bg_e000101",
                "bg_e000102/bg_e000102_night"
            ]
        );
        assert_eq!(refs.movies, ["school_refusal_opening"]);
        let voices: Vec<(&str, i64)> = refs
            .voices
            .iter()
            .map(|v| (v.id.as_str(), v.character2d_id))
            .collect();
        assert_eq!(
            voices,
            [
                ("voice_18_01", 18),
                ("partvoice_17kanade_01", 17),
                ("voice_op_01", 0)
            ]
        );
        assert_eq!(refs.bgm, ["bgm00000", "bgm00021"]);
        assert_eq!(
            refs.se,
            [
                SeRef {
                    cue: "se00001".into(),
                    bundle: None
                },
                SeRef {
                    cue: "se_ev01".into(),
                    bundle: Some("event_story/event_x_2023/scenario_se".into())
                },
                SeRef {
                    cue: "se00600".into(),
                    bundle: None
                },
                SeRef {
                    cue: "se_event_x_2023_001".into(),
                    bundle: None
                },
            ]
        );
        let effects: Vec<(&str, &str)> = refs
            .effects
            .iter()
            .map(|e| (e.name.as_str(), e.bundle.as_str()))
            .collect();
        assert_eq!(
            effects,
            [
                ("fx_rain", "scenario/effect/rain"),
                ("hologram", "scenario/effect/hologram"),
            ]
        );
        assert_eq!(refs.music_videos, ["5"]);
    }

    #[test]
    fn warns_about_unknown_effect_types_and_characters() {
        let refs = extract(&scenario()).unwrap();
        let kinds: Vec<WarningKind> = refs.warnings.iter().map(|w| w.kind).collect();
        assert_eq!(
            kinds,
            [
                WarningKind::UnknownCharacter,
                WarningKind::BundleMissing,
                WarningKind::UnknownEffectType
            ]
        );
        assert!(refs.warnings[0].detail.contains("99"));
        assert!(refs.warnings[1].detail.contains("fx_nothing"));
        assert!(refs.warnings[2].detail.contains("SpecialEffectData[12]"));
    }

    #[test]
    fn music_video_id_and_missing_fields() {
        let refs = extract(&json!({"ScenarioId": "x", "EpisodeMusicVideoId": "12"})).unwrap();
        assert_eq!(refs.music_videos, ["12"]);
        assert!(refs.characters.is_empty() && refs.backgrounds.is_empty());
        assert_eq!(extract(&json!([])), Err(ReferenceError::NotAnObject));
        assert_eq!(extract(&json!({})).unwrap().scenario_id, "");
    }
}
