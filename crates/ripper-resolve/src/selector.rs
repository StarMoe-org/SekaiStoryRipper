//! Story selectors: which episodes a `plan` / `rip` command works on.
//!
//! | selector | picks |
//! |---|---|
//! | `all` | every episode |
//! | `unit:all`, `event:all`, `card:all`, `special:all` | every episode of that type |
//! | `unit:<chapterAssetbundleName>[/<episodes>]` | a main story chapter |
//! | `event:<eventId>[/<episodes>]` | an event story |
//! | `special:<specialStoryId>[/<episodes>]` | a special story |
//! | `card:<cardId>[/first\|second\|<cardEpisodeId>]` | a card's side stories |
//! | `scenario:<ScenarioId>` | every episode playing that scenario |
//!
//! `<episodes>` is an `episodeNo` or an inclusive range `a-b`.

use std::fmt;
use std::str::FromStr;

use ripper_format::episode::StoryType;

use crate::catalog::Episode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    All,
    AllOf(StoryType),
    /// Unit/event/special: `story` is the chapter `assetbundleName` or the numeric story id.
    Story {
        story_type: StoryType,
        story: String,
        episodes: Option<EpisodeRange>,
    },
    Card {
        card_id: i64,
        part: Option<CardPart>,
    },
    Scenario(String),
}

/// Inclusive `episodeNo` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeRange {
    pub first: i64,
    pub last: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPart {
    First,
    Second,
    /// A `cardEpisodes.id`.
    EpisodeId(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    #[error("selector {0:?}: expected <type>:<id>[/<episode>], `scenario:<id>` or `all`")]
    Syntax(String),
    #[error("selector {0:?}: unknown story type (unit, event, card, special, scenario)")]
    UnknownType(String),
    #[error("selector {selector:?}: {part:?} is not a number")]
    NotANumber { selector: String, part: String },
}

fn number(selector: &str, part: &str) -> Result<i64, SelectorError> {
    part.parse().map_err(|_| SelectorError::NotANumber {
        selector: selector.into(),
        part: part.into(),
    })
}

fn range(selector: &str, part: &str) -> Result<EpisodeRange, SelectorError> {
    match part.split_once('-') {
        Some((first, last)) => Ok(EpisodeRange {
            first: number(selector, first)?,
            last: number(selector, last)?,
        }),
        None => {
            let no = number(selector, part)?;
            Ok(EpisodeRange {
                first: no,
                last: no,
            })
        }
    }
}

impl FromStr for Selector {
    type Err = SelectorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "all" {
            return Ok(Self::All);
        }
        let syntax = || SelectorError::Syntax(s.into());
        let (kind, rest) = s.split_once(':').ok_or_else(syntax)?;
        if rest.is_empty() {
            return Err(syntax());
        }
        if kind == "scenario" {
            return Ok(Self::Scenario(rest.into()));
        }
        let story_type = match kind {
            "unit" => StoryType::Unit,
            "event" => StoryType::Event,
            "card" => StoryType::Card,
            "special" => StoryType::Special,
            _ => return Err(SelectorError::UnknownType(s.into())),
        };
        if rest == "all" {
            return Ok(Self::AllOf(story_type));
        }
        let (story, episode) = match rest.split_once('/') {
            Some((story, episode)) if !story.is_empty() && !episode.is_empty() => {
                (story, Some(episode))
            }
            Some(_) => return Err(syntax()),
            None => (rest, None),
        };
        if story_type == StoryType::Card {
            let part = episode
                .map(|p| match p {
                    "first" => Ok(CardPart::First),
                    "second" => Ok(CardPart::Second),
                    id => number(s, id).map(CardPart::EpisodeId),
                })
                .transpose()?;
            return Ok(Self::Card {
                card_id: number(s, story)?,
                part,
            });
        }
        if story_type != StoryType::Unit {
            number(s, story)?;
        }
        Ok(Self::Story {
            story_type,
            story: story.into(),
            episodes: episode.map(|e| range(s, e)).transpose()?,
        })
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("all"),
            Self::AllOf(t) => write!(f, "{}:all", t.as_str()),
            Self::Story {
                story_type,
                story,
                episodes,
            } => {
                write!(f, "{}:{story}", story_type.as_str())?;
                match episodes {
                    Some(r) if r.first == r.last => write!(f, "/{}", r.first),
                    Some(r) => write!(f, "/{}-{}", r.first, r.last),
                    None => Ok(()),
                }
            }
            Self::Card { card_id, part } => {
                write!(f, "card:{card_id}")?;
                match part {
                    Some(CardPart::First) => f.write_str("/first"),
                    Some(CardPart::Second) => f.write_str("/second"),
                    Some(CardPart::EpisodeId(id)) => write!(f, "/{id}"),
                    None => Ok(()),
                }
            }
            Self::Scenario(id) => write!(f, "scenario:{id}"),
        }
    }
}

impl Selector {
    pub fn matches(&self, episode: &Episode) -> bool {
        let story = &episode.story;
        match self {
            Self::All => true,
            Self::AllOf(t) => story.story_type == *t,
            Self::Story {
                story_type,
                story: key,
                episodes,
            } => {
                story.story_type == *story_type
                    && episode.story_key == *key
                    && episodes.is_none_or(|r| (r.first..=r.last).contains(&story.episode_no))
            }
            Self::Card { card_id, part } => {
                story.story_type == StoryType::Card
                    && story.story_id == *card_id
                    && match part {
                        None => true,
                        Some(CardPart::First) => story.episode_no == 1,
                        Some(CardPart::Second) => story.episode_no == 2,
                        Some(CardPart::EpisodeId(id)) => story.episode_id == *id,
                    }
            }
            Self::Scenario(id) => story.scenario_id == *id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, tests::masterdata};

    fn picks(selector: &str) -> Vec<String> {
        let catalog = Catalog::new(&masterdata());
        let selector: Selector = selector.parse().unwrap();
        catalog
            .select(&selector)
            .into_iter()
            .map(|e| e.story.selector.clone())
            .collect()
    }

    #[test]
    fn parses_and_prints_every_form() {
        for s in [
            "all",
            "unit:all",
            "event:all",
            "card:all",
            "special:all",
            "unit:school-refusal-story-chapter",
            "unit:school-refusal-story-chapter/3",
            "event:120/1-4",
            "card:7",
            "card:7/first",
            "card:7/second",
            "card:7/2",
            "special:2/1",
            "scenario:nightcode_01_01",
        ] {
            let parsed: Selector = s.parse().unwrap();
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn rejects_malformed_selectors() {
        for s in [
            "",
            "unit",
            "unit:",
            "unit:x/",
            "unit:/3",
            "movie:1",
            "scenario:",
        ] {
            assert!(s.parse::<Selector>().is_err(), "{s}");
        }
        assert!(matches!(
            "event:abc".parse::<Selector>(),
            Err(SelectorError::NotANumber { .. })
        ));
        assert!(matches!(
            "card:7/third".parse::<Selector>(),
            Err(SelectorError::NotANumber { .. })
        ));
        assert!(matches!(
            "unit:x/1-y".parse::<Selector>(),
            Err(SelectorError::NotANumber { .. })
        ));
    }

    #[test]
    fn selects_episodes() {
        assert_eq!(picks("all").len(), 9);
        assert_eq!(picks("unit:all").len(), 3);
        assert_eq!(
            picks("unit:school-refusal-story-chapter/2-3"),
            [
                "unit:school-refusal-story-chapter/2",
                "unit:school-refusal-story-chapter/3"
            ]
        );
        assert_eq!(picks("event:120"), ["event:120/1", "event:120/2"]);
        assert_eq!(picks("event:121"), Vec::<String>::new());
        assert_eq!(picks("card:7/first"), ["card:7/first"]);
        assert_eq!(picks("card:7/2"), ["card:7/second"]);
        assert_eq!(picks("card:7").len(), 2);
        assert_eq!(picks("special:3/1"), ["special:3/1"]);
        assert_eq!(
            picks("scenario:nightcode_01_01"),
            ["unit:school-refusal-story-chapter/2"]
        );
    }
}
