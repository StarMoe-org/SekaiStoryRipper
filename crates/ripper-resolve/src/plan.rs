//! Phase 1: the bundles one episode needs, checked against the manifest.
//!
//! Runs after the scenario was found (see [`scenario_bundle_candidates`]) and its references
//! extracted, before anything else is downloaded. Every reference becomes bundle names by
//! [`crate::rules`] and [`crate::live2d`]; names the manifest does not have are left out of
//! [`Plan::bundles`] and reported as warnings (ADR-0009).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::BuildHasher;

use ripper_format::episode::{StoryInfo, Warning, WarningKind};
use serde::Serialize;

use crate::catalog::{Character2ds, Episode};
use crate::live2d;
use crate::references::{CharacterRefs, ScenarioRefs};
use crate::rules;

/// Placeholder of [`PlanOptions::voice_bundle_templates`].
pub const SCENARIO_ID_PLACEHOLDER: &str = "{scenarioId}";

/// The bundle names of a CDN manifest, and optionally their dependencies.
pub trait Manifest {
    fn contains(&self, bundle: &str) -> bool;

    /// Direct dependencies of a bundle (e.g. `scenario/effect/*` → `shader/particles`).
    fn dependencies(&self, _bundle: &str) -> Vec<String> {
        Vec::new()
    }
}

impl<S: BuildHasher> Manifest for HashSet<String, S> {
    fn contains(&self, bundle: &str) -> bool {
        HashSet::contains(self, bundle)
    }
}

impl Manifest for BTreeSet<String> {
    fn contains(&self, bundle: &str) -> bool {
        BTreeSet::contains(self, bundle)
    }
}

/// Bundle name → dependencies.
impl Manifest for BTreeMap<String, Vec<String>> {
    fn contains(&self, bundle: &str) -> bool {
        self.contains_key(bundle)
    }

    fn dependencies(&self, bundle: &str) -> Vec<String> {
        self.get(bundle).cloned().unwrap_or_default()
    }
}

/// Cue name (matched exactly) → bundles known to hold it, e.g. from scanning every part-voice
/// ACB. Searched after the rule-derived bundles.
pub type CueIndex = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOptions {
    /// Searched, in order, for an SE cue no rule places (after the episode's own SE bundles).
    pub fallback_se_bundles: Vec<String>,
    /// Extra voice bundles searched after the scenario's own; [`SCENARIO_ID_PLACEHOLDER`] is
    /// replaced by the masterdata `scenarioId`. Candidates missing from the manifest are skipped
    /// silently. Empty by default: `vs<id>` bundles are just the voices of scenarios named so.
    pub voice_bundle_templates: Vec<String>,
    /// Extra bundles per `VoiceId` (the part-voice fallback).
    pub voice_cue_index: CueIndex,
    /// Extra bundles per SE cue.
    pub se_cue_index: CueIndex,
    /// `Character2dId` → motion bundle, consulted before the game rule (emergency override for a
    /// game update; the 6.4.0 client itself has no exception table).
    pub motion_bundle_overrides: BTreeMap<i64, String>,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            fallback_se_bundles: rules::FALLBACK_SE_BUNDLES
                .iter()
                .map(|&b| b.to_owned())
                .collect(),
            voice_bundle_templates: Vec::new(),
            voice_cue_index: CueIndex::new(),
            se_cue_index: CueIndex::new(),
            motion_bundle_overrides: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub story: StoryInfo,
    pub scenario_bundle: String,
    /// Every bundle to fetch and unpack, sorted, with manifest dependencies; only names the
    /// manifest has.
    pub bundles: Vec<String>,
    pub characters: Vec<PlannedCharacter>,
    pub backgrounds: Vec<PlannedBackground>,
    /// Movie name → bundle (only bundles in the manifest).
    pub movies: Vec<(String, String)>,
    /// BGM name → bundle (only bundles in the manifest).
    pub bgm: Vec<(String, String)>,
    pub se: Vec<PlannedCue>,
    pub voices: Vec<PlannedCue>,
    /// Effect name → bundle (only bundles in the manifest).
    pub effects: Vec<(String, String)>,
    pub music_videos: Vec<String>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCharacter {
    pub id: i64,
    pub asset_name: Option<String>,
    pub motion_bundle: Option<String>,
    /// `(CostumeType, model bundle if in the manifest)`, first-appearance order.
    pub costumes: Vec<(String, Option<String>)>,
    pub motions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedBackground {
    /// Key in the episode index (see `BackgroundRef::key`).
    pub key: String,
    pub bundle: String,
    /// File stem to look for inside the bundle.
    pub file: String,
}

/// An SE or voice cue and where to look for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCue {
    pub cue: String,
    /// Bundles to search for the cue, in order (only bundles in the manifest).
    pub bundles: Vec<String>,
}

/// The episode's scenario bundle candidates that the manifest has, in order. The CLI fetches and
/// unpacks them one by one and takes the first whose record holds `<scenarioId>.json`
/// (`index::find_scenario_file`); when none does, the episode has no scenario
/// ([`scenario_missing`]).
pub fn scenario_bundle_candidates<'a>(
    episode: &'a Episode,
    manifest: &impl Manifest,
) -> Vec<&'a str> {
    episode
        .scenario_bundles
        .iter()
        .map(String::as_str)
        .filter(|b| manifest.contains(b))
        .collect()
}

/// The warning for an episode whose scenario is in none of its candidate bundles.
pub fn scenario_missing(episode: &Episode) -> Warning {
    Warning::new(
        WarningKind::BundleMissing,
        format!(
            "{}: scenario {} is in none of {:?}",
            episode.story.selector, episode.story.scenario_id, episode.scenario_bundles
        ),
    )
}

struct Planner<'m, M> {
    manifest: &'m M,
    bundles: BTreeSet<String>,
    warnings: Vec<Warning>,
}

impl<M: Manifest> Planner<'_, M> {
    /// Adds the bundle when the manifest has it; otherwise warns with `kind`.
    fn require(&mut self, bundle: &str, kind: WarningKind, why: &str) -> bool {
        if self.manifest.contains(bundle) {
            self.bundles.insert(bundle.to_owned());
            true
        } else {
            self.warnings.push(Warning::new(
                kind,
                format!("{bundle} ({why}) is not in the manifest"),
            ));
            false
        }
    }

    /// Appends the candidates the manifest has to `found` (and the plan), skipping the others.
    fn optional<'c>(
        &mut self,
        found: &mut Vec<String>,
        candidates: impl IntoIterator<Item = &'c String>,
    ) {
        for bundle in candidates {
            if self.manifest.contains(bundle) && !found.contains(bundle) {
                self.bundles.insert(bundle.clone());
                found.push(bundle.clone());
            }
        }
    }

    fn add_dependencies(&mut self) {
        let mut queue: Vec<String> = self.bundles.iter().cloned().collect();
        while let Some(bundle) = queue.pop() {
            for dependency in self.manifest.dependencies(&bundle) {
                if self.bundles.contains(&dependency) {
                    continue;
                }
                let why = format!("dependency of {bundle}");
                if self.require(&dependency, WarningKind::BundleMissing, &why) {
                    queue.push(dependency);
                }
            }
        }
    }

    fn character(
        &mut self,
        refs: &CharacterRefs,
        character2ds: &Character2ds,
        options: &PlanOptions,
    ) -> PlannedCharacter {
        // The download builder looks the master row up with ids <= 1 taken as 1 (§1.1).
        let master = character2ds.get(&live2d::download_character2d_id(refs.id));
        let asset_name = master.and_then(|m| m.asset_name.clone());
        let id = refs.id;
        let motion_bundle = if let Some(bundle) = options.motion_bundle_overrides.get(&id) {
            let why = format!("override for Character2dId {id}");
            self.require(bundle, WarningKind::UnresolvedMotionBundle, &why)
                .then(|| bundle.clone())
        } else {
            match (master, &asset_name) {
                (None, _) => {
                    self.warnings.push(Warning::new(
                        WarningKind::MasterManifestMismatch,
                        format!(
                            "Character2dId {id} is not in character2ds: masterdata older than \
                             the scenario/manifest"
                        ),
                    ));
                    None
                }
                (Some(_), None) => {
                    self.warnings.push(Warning::new(
                        WarningKind::UnresolvedMotionBundle,
                        format!("Character2dId {id}: character2ds row has no assetName"),
                    ));
                    None
                }
                (Some(_), Some(name)) => {
                    let bundle = live2d::motion_bundle(name);
                    let why = format!("motions of Character2dId {id}");
                    self.require(&bundle, WarningKind::UnresolvedMotionBundle, &why)
                        .then_some(bundle)
                }
            }
        };
        let costumes = refs
            .costumes
            .iter()
            .map(|costume| {
                let bundle = live2d::model_bundle(costume);
                let why = format!("model of Character2dId {id}");
                let found = self.require(&bundle, WarningKind::ModelBundleMissing, &why);
                (costume.clone(), found.then_some(bundle))
            })
            .collect();
        PlannedCharacter {
            id,
            asset_name,
            motion_bundle,
            costumes,
            motions: refs.motions.clone(),
        }
    }

    fn voices(
        &mut self,
        episode: &Episode,
        scenario_bundle: &str,
        refs: &ScenarioRefs,
        character2ds: &Character2ds,
        options: &PlanOptions,
    ) -> Vec<PlannedCue> {
        if refs.voices.is_empty() {
            return Vec::new();
        }
        let scenario_id = &episode.story.scenario_id;
        let own = episode.voice_bundle(scenario_bundle);
        let mut shared = Vec::new();
        let templates: Vec<String> = options
            .voice_bundle_templates
            .iter()
            .map(|t| t.replace(SCENARIO_ID_PLACEHOLDER, scenario_id))
            .collect();
        self.optional(&mut shared, std::iter::once(&own).chain(&templates));
        // Scenarios without their own voice bundle exist (VS cards, some specials): fine as long
        // as every line is a part voice.
        if !self.manifest.contains(&own) && refs.voices.iter().any(|v| !rules::is_part_voice(&v.id))
        {
            self.warnings.push(Warning::new(
                WarningKind::BundleMissing,
                format!("{own} (voices of {scenario_id}) is not in the manifest"),
            ));
        }

        let mut planned: Vec<PlannedCue> = Vec::new();
        for voice in &refs.voices {
            let slot = match planned.iter().position(|p| p.cue == voice.id) {
                Some(slot) => slot,
                None => {
                    planned.push(PlannedCue {
                        cue: voice.id.clone(),
                        bundles: shared.clone(),
                    });
                    planned.len() - 1
                }
            };
            let mut bundles = std::mem::take(&mut planned[slot].bundles);
            if rules::is_part_voice(&voice.id)
                && let Some(master) = character2ds.get(&voice.character2d_id)
                && let (Some(asset_name), Some(unit)) = (&master.asset_name, &master.unit)
            {
                let bundle = rules::part_voice_bundle(asset_name, unit);
                self.optional(&mut bundles, [&bundle]);
            }
            if let Some(indexed) = options.voice_cue_index.get(&voice.id) {
                self.optional(&mut bundles, indexed);
            }
            planned[slot].bundles = bundles;
        }
        for cue in &planned {
            if cue.bundles.is_empty() && rules::is_part_voice(&cue.cue) {
                self.warnings.push(Warning::new(
                    WarningKind::BundleMissing,
                    format!(
                        "part voice {:?}: no bundle in the manifest (own, part_voice rule, index)",
                        cue.cue
                    ),
                ));
            }
        }
        planned
    }

    fn se(
        &mut self,
        episode: &Episode,
        refs: &ScenarioRefs,
        options: &PlanOptions,
    ) -> Vec<PlannedCue> {
        let mut planned: Vec<PlannedCue> = Vec::new();
        for se in &refs.se {
            let mut bundles = Vec::new();
            if let Some(hint) = &se.bundle {
                let why = format!("SeBundleName of {}", se.cue);
                if self.require(hint, WarningKind::BundleMissing, &why) {
                    bundles.push(hint.clone());
                }
            }
            let by_rule = rules::se_bundles(&se.cue);
            if by_rule.is_empty() {
                self.optional(
                    &mut bundles,
                    episode
                        .se_bundles
                        .iter()
                        .chain(&options.fallback_se_bundles),
                );
            } else {
                self.optional(&mut bundles, &by_rule);
            }
            if let Some(indexed) = options.se_cue_index.get(&se.cue) {
                self.optional(&mut bundles, indexed);
            }
            if bundles.is_empty() {
                self.warnings.push(Warning::new(
                    WarningKind::BundleMissing,
                    format!("SE {:?}: none of {by_rule:?} is in the manifest", se.cue),
                ));
            }
            match planned.iter_mut().find(|p| p.cue == se.cue) {
                Some(existing) => {
                    for bundle in bundles {
                        if !existing.bundles.contains(&bundle) {
                            existing.bundles.push(bundle);
                        }
                    }
                }
                None => planned.push(PlannedCue {
                    cue: se.cue.clone(),
                    bundles,
                }),
            }
        }
        planned
    }
}

/// Plans one episode. `scenario_bundle` is the bundle its scenario was found in (see
/// [`scenario_bundle_candidates`]); `character2ds` comes from `Masterdata::character2d_index`.
pub fn plan(
    episode: &Episode,
    scenario_bundle: &str,
    refs: &ScenarioRefs,
    manifest: &impl Manifest,
    character2ds: &Character2ds,
    options: &PlanOptions,
) -> Plan {
    let mut planner = Planner {
        manifest,
        bundles: BTreeSet::new(),
        warnings: refs.warnings.clone(),
    };
    planner.require(scenario_bundle, WarningKind::BundleMissing, "scenario");

    let characters = refs
        .characters
        .iter()
        .map(|c| planner.character(c, character2ds, options))
        .collect();

    let mut backgrounds = Vec::new();
    for background in &refs.backgrounds {
        let bundle = rules::background_bundle(&background.name);
        if planner.require(&bundle, WarningKind::BundleMissing, "background") {
            backgrounds.push(PlannedBackground {
                key: background.key(),
                bundle,
                file: background.file.clone(),
            });
        }
    }

    let mut movies = Vec::new();
    for name in &refs.movies {
        let bundle = rules::movie_bundle(name);
        if planner.require(&bundle, WarningKind::BundleMissing, "movie") {
            movies.push((name.clone(), bundle));
        }
    }

    let mut bgm = Vec::new();
    for name in &refs.bgm {
        let bundle = rules::bgm_bundle(name);
        if planner.require(&bundle, WarningKind::BundleMissing, "bgm") {
            bgm.push((name.clone(), bundle));
        }
    }

    let mut effects = Vec::new();
    for effect in &refs.effects {
        let why = format!("effect {}", effect.name);
        if planner.require(&effect.bundle, WarningKind::BundleMissing, &why) {
            effects.push((effect.name.clone(), effect.bundle.clone()));
        }
    }

    let voices = planner.voices(episode, scenario_bundle, refs, character2ds, options);
    let se = planner.se(episode, refs, options);

    for id in &refs.music_videos {
        planner.warnings.push(Warning::new(
            WarningKind::OutOfScope,
            format!("music video {id} is not exported by v1"),
        ));
    }

    planner.add_dependencies();
    Plan {
        story: episode.story.clone(),
        scenario_bundle: scenario_bundle.to_owned(),
        bundles: planner.bundles.into_iter().collect(),
        characters,
        backgrounds,
        movies,
        bgm,
        se,
        voices,
        effects,
        music_videos: refs.music_videos.clone(),
        warnings: planner.warnings,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::catalog::{Catalog, Character2dInfo, tests::masterdata};
    use crate::references::{VoiceRef, extract, tests::scenario};

    pub(crate) fn manifest() -> BTreeMap<String, Vec<String>> {
        [
            "scenario/unitstory/school-refusal-story-chapter",
            "live2d/model/17kanade_normal",
            "live2d/model/17kanade_cloth001",
            "live2d/model/18mafuyu_normal",
            "live2d/motion/17kanade_motion_base",
            "scenario/background/bg_a000000",
            "scenario/background/bg_h000104",
            "scenario/background/bg_e000101",
            "scenario/background/bg_e000102",
            "scenario/movie/school_refusal_opening",
            "sound/scenario/bgm/bgm00000",
            "sound/scenario/voice/nightcode_01_01",
            "sound/scenario/part_voice/17kanade_school_refusal",
            "sound/scenario/se/se",
            "sound/scenario/se/se_pack00001",
            "sound/scenario/se/se_pack00001_b",
            "event_story/event_x_2023/scenario_se",
            "scenario/effect/hologram",
            "shader/particles",
        ]
        .into_iter()
        .map(|b| {
            let deps = if b == "scenario/effect/hologram" {
                vec!["shader/particles".to_owned()]
            } else {
                Vec::new()
            };
            (b.to_owned(), deps)
        })
        .collect()
    }

    pub(crate) fn character2ds() -> Character2ds {
        // 18 has a row without assetName; 5 has no row at all.
        BTreeMap::from([
            (17, Character2dInfo::named("17kanade", "school_refusal")),
            (
                18,
                Character2dInfo {
                    asset_name: None,
                    unit: Some("school_refusal".into()),
                },
            ),
        ])
    }

    fn episode(selector: &str) -> Episode {
        Catalog::new(&masterdata())
            .episodes
            .into_iter()
            .find(|e| e.story.selector == selector)
            .unwrap()
    }

    pub(crate) fn test_plan() -> Plan {
        let episode = episode("unit:school-refusal-story-chapter/2");
        let manifest = manifest();
        let bundle = scenario_bundle_candidates(&episode, &manifest)[0].to_owned();
        let refs = extract(&scenario()).unwrap();
        plan(
            &episode,
            &bundle,
            &refs,
            &manifest,
            &character2ds(),
            &PlanOptions::default(),
        )
    }

    fn details(plan: &Plan, kind: WarningKind) -> Vec<&str> {
        plan.warnings
            .iter()
            .filter(|w| w.kind == kind)
            .map(|w| w.detail.as_str())
            .collect()
    }

    fn cue<'p>(cues: &'p [PlannedCue], name: &str) -> &'p [String] {
        &cues.iter().find(|c| c.cue == name).unwrap().bundles
    }

    #[test]
    fn plans_bundles_from_every_reference_kind() {
        let plan = test_plan();
        assert_eq!(
            plan.scenario_bundle,
            "scenario/unitstory/school-refusal-story-chapter"
        );
        let expected: Vec<&str> = vec![
            "event_story/event_x_2023/scenario_se",
            "live2d/model/17kanade_cloth001",
            "live2d/model/17kanade_normal",
            "live2d/model/18mafuyu_normal",
            "live2d/motion/17kanade_motion_base",
            "scenario/background/bg_a000000",
            "scenario/background/bg_e000101",
            "scenario/background/bg_e000102",
            "scenario/background/bg_h000104",
            "scenario/effect/hologram",
            "scenario/movie/school_refusal_opening",
            "scenario/unitstory/school-refusal-story-chapter",
            "shader/particles",
            "sound/scenario/bgm/bgm00000",
            "sound/scenario/part_voice/17kanade_school_refusal",
            "sound/scenario/se/se_pack00001",
            "sound/scenario/se/se_pack00001_b",
            "sound/scenario/voice/nightcode_01_01",
        ];
        assert_eq!(plan.bundles, expected);
        let kanade = &plan.characters[0];
        assert_eq!(
            kanade.motion_bundle.as_deref(),
            Some("live2d/motion/17kanade_motion_base")
        );
        assert_eq!(kanade.costumes[2], ("17kanade_black".to_owned(), None));
    }

    #[test]
    fn places_se_cues_by_rule() {
        let plan = test_plan();
        assert_eq!(cue(&plan.se, "se00001"), ["sound/scenario/se/se_pack00001"]);
        assert_eq!(
            cue(&plan.se, "se00600"),
            ["sound/scenario/se/se_pack00001_b"]
        );
        assert_eq!(
            cue(&plan.se, "se_event_x_2023_001"),
            ["event_story/event_x_2023/scenario_se"]
        );
        // No rule: `SeBundleName` when set, then the fallback packs (never `sound/scenario/se/se`).
        assert_eq!(
            cue(&plan.se, "se_ev01"),
            [
                "event_story/event_x_2023/scenario_se",
                "sound/scenario/se/se_pack00001",
                "sound/scenario/se/se_pack00001_b"
            ]
        );
    }

    #[test]
    fn searches_voices_own_bundle_then_part_voice_then_index() {
        let mut options = PlanOptions::default();
        options.voice_cue_index.insert(
            "partvoice_17kanade_01".into(),
            vec![
                "sound/scenario/part_voice/17kanade_school_refusal".into(),
                "sound/scenario/voice/part_voice_v2_17kanade_x".into(),
            ],
        );
        let mut manifest = manifest();
        manifest.insert(
            "sound/scenario/voice/part_voice_v2_17kanade_x".into(),
            vec![],
        );
        let episode = episode("unit:school-refusal-story-chapter/2");
        let refs = extract(&scenario()).unwrap();
        let plan = plan(
            &episode,
            &episode.scenario_bundles[0],
            &refs,
            &manifest,
            &character2ds(),
            &options,
        );
        assert_eq!(
            cue(&plan.voices, "voice_18_01"),
            ["sound/scenario/voice/nightcode_01_01"]
        );
        assert_eq!(
            cue(&plan.voices, "partvoice_17kanade_01"),
            [
                "sound/scenario/voice/nightcode_01_01",
                "sound/scenario/part_voice/17kanade_school_refusal",
                "sound/scenario/voice/part_voice_v2_17kanade_x"
            ]
        );
    }

    #[test]
    fn part_voices_alone_need_no_own_voice_bundle() {
        let episode = episode("card:7/first");
        let refs = ScenarioRefs {
            voices: vec![
                VoiceRef {
                    id: "partvoice_17kanade_01".into(),
                    character2d_id: 17,
                },
                VoiceRef {
                    id: "partvoice_18mafuyu_01".into(),
                    character2d_id: 18,
                },
            ],
            ..ScenarioRefs::default()
        };
        let plan = plan(
            &episode,
            &episode.scenario_bundles[0],
            &refs,
            &manifest(),
            &character2ds(),
            &PlanOptions::default(),
        );
        assert_eq!(
            cue(&plan.voices, "partvoice_17kanade_01"),
            ["sound/scenario/part_voice/17kanade_school_refusal"]
        );
        let missing = details(&plan, WarningKind::BundleMissing);
        // The card's own voice bundle is absent, but only part voices are used.
        assert!(
            !missing.iter().any(|d| d.contains("card_scenario/voice")),
            "{missing:?}"
        );
        assert!(missing.contains(
            &"part voice \"partvoice_18mafuyu_01\": no bundle in the manifest (own, part_voice rule, index)"
        ));
    }

    #[test]
    fn missing_own_voice_bundle_is_reported_for_ordinary_voices() {
        let episode = episode("unit:school-refusal-story-chapter/3");
        let refs = extract(&scenario()).unwrap();
        let plan = plan(
            &episode,
            &episode.scenario_bundles[0],
            &refs,
            &manifest(),
            &character2ds(),
            &PlanOptions::default(),
        );
        assert!(details(&plan, WarningKind::BundleMissing).contains(
            &"sound/scenario/voice/nightcode_01_02 (voices of nightcode_01_02) is not in the manifest"
        ));
        assert!(cue(&plan.voices, "voice_18_01").is_empty());
    }

    #[test]
    fn reports_each_warning_kind() {
        let plan = test_plan();
        assert_eq!(
            details(&plan, WarningKind::ModelBundleMissing),
            [
                "live2d/model/17kanade_black (model of Character2dId 17) is not in the manifest",
                "live2d/model/005_casual (model of Character2dId 5) is not in the manifest"
            ]
        );
        assert_eq!(
            details(&plan, WarningKind::UnresolvedMotionBundle),
            ["Character2dId 18: character2ds row has no assetName"]
        );
        assert_eq!(details(&plan, WarningKind::MasterManifestMismatch).len(), 1);
        let missing = details(&plan, WarningKind::BundleMissing);
        assert!(missing.contains(&"sound/scenario/bgm/bgm00021 (bgm) is not in the manifest"));
        assert!(missing.contains(&"scenario/effect/rain (effect fx_rain) is not in the manifest"));
        assert!(missing.iter().any(|d| d.contains("fx_nothing")));
        assert_eq!(details(&plan, WarningKind::OutOfScope).len(), 1);
        assert_eq!(details(&plan, WarningKind::UnknownEffectType).len(), 1);
        assert_eq!(details(&plan, WarningKind::UnknownCharacter).len(), 1);
    }

    #[test]
    fn motion_bundle_override_wins_and_missing_bundle_is_unresolved() {
        let episode = episode("unit:school-refusal-story-chapter/2");
        let refs = extract(&scenario()).unwrap();
        let options = PlanOptions {
            motion_bundle_overrides: BTreeMap::from([(
                18,
                "live2d/motion/17kanade_motion_base".into(),
            )]),
            ..PlanOptions::default()
        };
        let mut character2ds = character2ds();
        character2ds.insert(17, Character2dInfo::named("v2_17kanade", "school_refusal"));
        let plan = plan(&episode, "x", &refs, &manifest(), &character2ds, &options);
        assert_eq!(
            plan.characters[1].motion_bundle.as_deref(),
            Some("live2d/motion/17kanade_motion_base")
        );
        assert_eq!(plan.characters[0].motion_bundle, None);
        assert_eq!(
            details(&plan, WarningKind::UnresolvedMotionBundle),
            [
                "live2d/motion/v2_17kanade_motion_base (motions of Character2dId 17) is not in the manifest"
            ]
        );
        assert!(
            details(&plan, WarningKind::BundleMissing)
                .contains(&"x (scenario) is not in the manifest")
        );
    }

    #[test]
    fn scenario_candidates_are_filtered_by_the_manifest() {
        let op = episode("special:2/1");
        let manifest = BTreeSet::from(["scenario/special/special-story".to_owned()]);
        assert_eq!(
            scenario_bundle_candidates(&op, &manifest),
            ["scenario/special/special-story"]
        );
        assert!(scenario_bundle_candidates(&op, &BTreeSet::new()).is_empty());
        let warning = scenario_missing(&op);
        assert_eq!(warning.kind, WarningKind::BundleMissing);
        assert!(
            warning
                .detail
                .starts_with("special:2/1: scenario op_01 is in none of")
        );
    }

    #[test]
    fn character_id_zero_uses_master_row_one() {
        let refs = ScenarioRefs {
            scenario_id: "op".into(),
            characters: vec![CharacterRefs {
                id: 0,
                costumes: vec!["01ichika_normal".into()],
                motions: vec![],
            }],
            ..ScenarioRefs::default()
        };
        let manifest = BTreeSet::from(["live2d/motion/01ichika_motion_base".to_owned()]);
        let character2ds = BTreeMap::from([(1, Character2dInfo::named("01ichika", "light_sound"))]);
        let plan = plan(
            &episode("unit:school-refusal-story-chapter/1"),
            "x",
            &refs,
            &manifest,
            &character2ds,
            &PlanOptions::default(),
        );
        assert_eq!(
            plan.characters[0].motion_bundle.as_deref(),
            Some("live2d/motion/01ichika_motion_base")
        );
    }
}
