//! Masterdata → the list of story episodes and the bundles each one starts from.
//!
//! Tables are read with typed structs that ignore unknown fields, so a newer masterdata keeps
//! parsing. Bundle names come from [`crate::rules`], and the scenario of an episode is always
//! named by the masterdata `scenarioId` (never by the `ScenarioId` inside the scenario).

use std::collections::{BTreeMap, HashMap};

use ripper_format::episode::{StoryInfo, StoryType};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::rules;

/// Masterdata tables the catalog reads, by file stem (`<name>.json`).
pub const TABLES: [&str; 6] = [
    "unitStories",
    "eventStories",
    "cardEpisodes",
    "cards",
    "specialStories",
    "character2ds",
];

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("masterdata table {table}: {source}")]
    Json {
        table: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unknown masterdata table {0:?}")]
    UnknownTable(String),
}

// ---- masterdata --------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitStory {
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub chapters: Vec<UnitStoryChapter>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitStoryChapter {
    pub id: i64,
    pub assetbundle_name: String,
    #[serde(default)]
    pub episodes: Vec<StoryEpisode>,
}

/// An episode row of `unitStories`, `eventStories` or `specialStories`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryEpisode {
    pub id: i64,
    pub episode_no: i64,
    #[serde(default)]
    pub title: String,
    pub scenario_id: String,
    #[serde(default)]
    pub assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventStory {
    pub id: i64,
    pub event_id: i64,
    pub assetbundle_name: String,
    #[serde(default)]
    pub event_story_episodes: Vec<StoryEpisode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardEpisode {
    pub id: i64,
    pub card_id: i64,
    #[serde(default)]
    pub title: String,
    pub scenario_id: String,
    /// `first_part` / `second_part`.
    #[serde(default)]
    pub card_episode_part_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub assetbundle_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecialStory {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub assetbundle_name: String,
    #[serde(default)]
    pub episodes: Vec<StoryEpisode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character2d {
    pub id: i64,
    /// Absent (haruki drops null keys) for mobs and other non-Live2D entries.
    #[serde(default)]
    pub asset_name: Option<String>,
    /// e.g. `school_refusal`, `none`; part-voice bundles are named after it.
    #[serde(default)]
    pub unit: Option<String>,
}

/// The masterdata tables of [`TABLES`]; a table that was not loaded stays empty.
#[derive(Debug, Clone, Default)]
pub struct Masterdata {
    pub unit_stories: Vec<UnitStory>,
    pub event_stories: Vec<EventStory>,
    pub card_episodes: Vec<CardEpisode>,
    pub cards: Vec<Card>,
    pub special_stories: Vec<SpecialStory>,
    pub character2ds: Vec<Character2d>,
}

fn parse<T: DeserializeOwned>(table: &str, json: &[u8]) -> Result<Vec<T>, CatalogError> {
    serde_json::from_slice(json).map_err(|source| CatalogError::Json {
        table: table.into(),
        source,
    })
}

impl Masterdata {
    /// Parses one table of [`TABLES`] from its JSON file.
    pub fn load_table(&mut self, table: &str, json: &[u8]) -> Result<(), CatalogError> {
        match table {
            "unitStories" => self.unit_stories = parse(table, json)?,
            "eventStories" => self.event_stories = parse(table, json)?,
            "cardEpisodes" => self.card_episodes = parse(table, json)?,
            "cards" => self.cards = parse(table, json)?,
            "specialStories" => self.special_stories = parse(table, json)?,
            "character2ds" => self.character2ds = parse(table, json)?,
            _ => return Err(CatalogError::UnknownTable(table.into())),
        }
        Ok(())
    }

    /// `character2ds` by id. An id missing from the map has no row at all.
    pub fn character2d_index(&self) -> Character2ds {
        let non_empty = |s: &Option<String>| s.clone().filter(|n| !n.is_empty());
        self.character2ds
            .iter()
            .map(|c| {
                let info = Character2dInfo {
                    asset_name: non_empty(&c.asset_name),
                    unit: non_empty(&c.unit),
                };
                (c.id, info)
            })
            .collect()
    }
}

/// What the resolver needs of one `character2ds` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Character2dInfo {
    /// `None` when the row has no (or an empty) `assetName`.
    pub asset_name: Option<String>,
    pub unit: Option<String>,
}

impl Character2dInfo {
    pub fn named(asset_name: &str, unit: &str) -> Self {
        Self {
            asset_name: Some(asset_name.into()),
            unit: Some(unit.into()),
        }
    }
}

/// `character2ds` id → row, see [`Masterdata::character2d_index`].
pub type Character2ds = BTreeMap<i64, Character2dInfo>;

// ---- episodes ----------------------------------------------------------------------------------

/// One playable episode and where its scenario, voices and own SE live.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    pub story: StoryInfo,
    /// unit: the chapter's `assetbundleName` (what `unit:` selectors name); others: the story id.
    pub story_key: String,
    /// Candidate scenario bundles in order: the scenario is `<scenarioId>.json` in the first one
    /// that holds it (see `plan::scenario_bundle_candidates`).
    pub scenario_bundles: Vec<String>,
    /// Episode-specific SE bundles (an event's `scenario_se`), searched for cues no SE rule
    /// places, before `rules::FALLBACK_SE_BUNDLES`.
    pub se_bundles: Vec<String>,
}

impl Episode {
    /// The scenario's own voice bundle, given the bundle its scenario was found in.
    pub fn voice_bundle(&self, scenario_bundle: &str) -> String {
        rules::VoiceFamily::of_scenario_bundle(scenario_bundle)
            .voice_bundle(&self.story.scenario_id)
    }
}

/// Every episode of the masterdata, in table order.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub episodes: Vec<Episode>,
    /// Rows that could not become an episode (e.g. a card episode whose card is missing).
    pub skipped: Vec<String>,
}

fn selector(story_type: StoryType, story: &str, episode: &str) -> String {
    format!("{}:{story}/{episode}", story_type.as_str())
}

impl Catalog {
    pub fn new(masterdata: &Masterdata) -> Self {
        let mut catalog = Self::default();
        catalog.add_unit_stories(&masterdata.unit_stories);
        catalog.add_event_stories(&masterdata.event_stories);
        catalog.add_card_episodes(&masterdata.card_episodes, &masterdata.cards);
        catalog.add_special_stories(&masterdata.special_stories);
        catalog
    }

    fn add_unit_stories(&mut self, stories: &[UnitStory]) {
        for chapter in stories.iter().flat_map(|s| &s.chapters) {
            for episode in &chapter.episodes {
                self.episodes.push(Episode {
                    story: StoryInfo {
                        story_type: StoryType::Unit,
                        selector: selector(
                            StoryType::Unit,
                            &chapter.assetbundle_name,
                            &episode.episode_no.to_string(),
                        ),
                        story_id: chapter.id,
                        episode_no: episode.episode_no,
                        episode_id: episode.id,
                        title: episode.title.clone(),
                        scenario_id: episode.scenario_id.clone(),
                    },
                    story_key: chapter.assetbundle_name.clone(),
                    scenario_bundles: vec![rules::unit_scenario_bundle(&chapter.assetbundle_name)],
                    se_bundles: Vec::new(),
                });
            }
        }
    }

    fn add_event_stories(&mut self, stories: &[EventStory]) {
        for story in stories {
            for episode in &story.event_story_episodes {
                self.episodes.push(Episode {
                    story: StoryInfo {
                        story_type: StoryType::Event,
                        selector: selector(
                            StoryType::Event,
                            &story.event_id.to_string(),
                            &episode.episode_no.to_string(),
                        ),
                        story_id: story.event_id,
                        episode_no: episode.episode_no,
                        episode_id: episode.id,
                        title: episode.title.clone(),
                        scenario_id: episode.scenario_id.clone(),
                    },
                    story_key: story.event_id.to_string(),
                    scenario_bundles: vec![rules::event_scenario_bundle(&story.assetbundle_name)],
                    se_bundles: vec![rules::event_se_bundle(&story.assetbundle_name)],
                });
            }
        }
    }

    fn add_card_episodes(&mut self, episodes: &[CardEpisode], cards: &[Card]) {
        let cards: HashMap<i64, &Card> = cards.iter().map(|c| (c.id, c)).collect();
        let mut per_card: HashMap<i64, i64> = HashMap::new();
        for episode in episodes {
            let Some(card) = cards.get(&episode.card_id) else {
                self.skipped.push(format!(
                    "cardEpisodes {}: card {} not in cards",
                    episode.id, episode.card_id
                ));
                continue;
            };
            let ordinal = per_card.entry(episode.card_id).or_default();
            *ordinal += 1;
            let (episode_no, part) = match episode.card_episode_part_type.as_deref() {
                Some("first_part") => (1, "first".to_owned()),
                Some("second_part") => (2, "second".to_owned()),
                _ => (*ordinal, episode.id.to_string()),
            };
            self.episodes.push(Episode {
                story: StoryInfo {
                    story_type: StoryType::Card,
                    selector: selector(StoryType::Card, &episode.card_id.to_string(), &part),
                    story_id: episode.card_id,
                    episode_no,
                    episode_id: episode.id,
                    title: episode.title.clone(),
                    scenario_id: episode.scenario_id.clone(),
                },
                story_key: episode.card_id.to_string(),
                scenario_bundles: vec![rules::card_scenario_bundle(&card.assetbundle_name)],
                se_bundles: Vec::new(),
            });
        }
    }

    fn add_special_stories(&mut self, stories: &[SpecialStory]) {
        for story in stories {
            for episode in &story.episodes {
                self.episodes.push(Episode {
                    story: StoryInfo {
                        story_type: StoryType::Special,
                        selector: selector(
                            StoryType::Special,
                            &story.id.to_string(),
                            &episode.episode_no.to_string(),
                        ),
                        story_id: story.id,
                        episode_no: episode.episode_no,
                        episode_id: episode.id,
                        title: episode.title.clone(),
                        scenario_id: episode.scenario_id.clone(),
                    },
                    story_key: story.id.to_string(),
                    scenario_bundles: rules::special_scenario_bundles(
                        &episode.scenario_id,
                        &story.assetbundle_name,
                        episode.assetbundle_name.as_deref(),
                    ),
                    se_bundles: Vec::new(),
                });
            }
        }
    }

    /// The episodes a selector picks, in catalog order.
    pub fn select(&self, selector: &crate::selector::Selector) -> Vec<&Episode> {
        self.episodes
            .iter()
            .filter(|e| selector.matches(e))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Hand-written tables with the shape (and a few extra fields) of the haruki masterdata.
    pub(crate) fn masterdata() -> Masterdata {
        let mut md = Masterdata::default();
        let tables: [(&str, &str); 6] = [
            (
                "unitStories",
                r#"[{"unit":"school_refusal","seq":5,"chapters":[{"id":5,"unit":"school_refusal",
                   "chapterNo":1,"assetbundleName":"school-refusal-story-chapter","episodes":[
                   {"id":50000,"episodeNo":1,"episodeNoLabel":"序章","title":"prologue",
                    "scenarioId":"nightcode_01_00","assetbundleName":"story_nc_01_00"},
                   {"id":50001,"episodeNo":2,"title":"one","scenarioId":"nightcode_01_01"},
                   {"id":50002,"episodeNo":3,"title":"two","scenarioId":"nightcode_01_02"}]}]}]"#,
            ),
            (
                "eventStories",
                r#"[{"id":1,"eventId":120,"outline":"…","assetbundleName":"event_x_2023",
                   "eventStoryEpisodes":[
                   {"id":1200001,"eventStoryId":1,"episodeNo":1,"title":"e1","scenarioId":"event_120_01",
                    "episodeRewards":[]},
                   {"id":1200002,"eventStoryId":1,"episodeNo":2,"title":"e2","scenarioId":"event_120_02"}]}]"#,
            ),
            (
                "cardEpisodes",
                r#"[{"id":1,"cardId":7,"title":"up","scenarioId":"001007_ichika01","cardEpisodePartType":"first_part"},
                   {"id":2,"cardId":7,"title":"down","scenarioId":"001007_ichika02","cardEpisodePartType":"second_part"},
                   {"id":3,"cardId":999,"title":"orphan","scenarioId":"x"}]"#,
            ),
            (
                "cards",
                r#"[{"id":7,"characterId":1,"assetbundleName":"res001_no007","prefix":"p"}]"#,
            ),
            (
                "specialStories",
                r#"[{"id":2,"title":"sp","assetbundleName":"special-story","episodes":[
                   {"id":1,"specialStoryId":2,"episodeNo":1,"title":"op","scenarioId":"op_01",
                    "assetbundleName":"story_sp_ts_01_01"}]},
                   {"id":3,"title":"cd","assetbundleName":"special-story","episodes":[
                   {"id":2,"specialStoryId":3,"episodeNo":1,"title":"cd1","scenarioId":"1st_countdown_01",
                    "assetbundleName":"story_1st_countdown"}]}]"#,
            ),
            (
                "character2ds",
                r#"[{"id":17,"characterId":17,"characterType":"game_character","assetName":"17kanade","unit":"school_refusal"},
                   {"id":26,"characterId":21,"characterType":"game_character","assetName":"21miku","unit":"school_refusal"},
                   {"id":900000,"characterId":900000,"characterType":"mob","unit":"none"}]"#,
            ),
        ];
        for (table, json) in tables {
            md.load_table(table, json.as_bytes()).unwrap();
        }
        md
    }

    #[test]
    fn builds_every_story_type_with_its_bundles() {
        let catalog = Catalog::new(&masterdata());
        let by_selector: HashMap<&str, &Episode> = catalog
            .episodes
            .iter()
            .map(|e| (e.story.selector.as_str(), e))
            .collect();
        assert_eq!(catalog.episodes.len(), 9);

        let unit = by_selector["unit:school-refusal-story-chapter/2"];
        assert_eq!(unit.story.scenario_id, "nightcode_01_01");
        assert_eq!((unit.story.story_id, unit.story.episode_id), (5, 50001));
        assert_eq!(
            unit.scenario_bundles,
            ["scenario/unitstory/school-refusal-story-chapter"]
        );
        assert_eq!(
            unit.voice_bundle(&unit.scenario_bundles[0]),
            "sound/scenario/voice/nightcode_01_01"
        );

        let event = by_selector["event:120/2"];
        assert_eq!(
            event.scenario_bundles,
            ["event_story/event_x_2023/scenario"]
        );
        assert_eq!(event.se_bundles, ["event_story/event_x_2023/scenario_se"]);

        let card = by_selector["card:7/second"];
        assert_eq!(card.story.episode_no, 2);
        assert_eq!(card.scenario_bundles, ["character/member/res001_no007"]);
        assert_eq!(
            card.voice_bundle(&card.scenario_bundles[0]),
            "sound/card_scenario/voice/001007_ichika02"
        );

        let special = by_selector["special:2/1"];
        assert_eq!(
            special.scenario_bundles,
            [
                "scenario/special/story_sp_ts_01_01",
                "scenario/special/special-story"
            ]
        );
        assert_eq!(catalog.skipped.len(), 1, "{:?}", catalog.skipped);
    }

    #[test]
    fn character2ds_keep_missing_asset_names_apart_from_missing_rows() {
        let names = masterdata().character2d_index();
        assert_eq!(
            names[&17],
            Character2dInfo::named("17kanade", "school_refusal")
        );
        assert_eq!(names[&900000].asset_name, None);
        assert_eq!(names[&900000].unit.as_deref(), Some("none"));
        assert!(!names.contains_key(&18));
    }

    #[test]
    fn rejects_unknown_tables_and_bad_json() {
        let mut md = Masterdata::default();
        assert!(matches!(
            md.load_table("musics", b"[]"),
            Err(CatalogError::UnknownTable(_))
        ));
        assert!(matches!(
            md.load_table("cards", b"{}"),
            Err(CatalogError::Json { .. })
        ));
    }
}
