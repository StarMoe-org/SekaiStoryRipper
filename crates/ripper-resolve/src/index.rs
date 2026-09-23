//! Phase 2: the episode index, from the plan and the unpack records of its bundles.
//!
//! Files are found through the records' container paths (what the game looks up), and every
//! path written is relative to `library/`. Audio goes through a caller-supplied cue lookup so
//! this crate does not depend on the ACB index format.

use std::collections::{BTreeMap, HashMap, HashSet};

use ripper_format::episode::{
    self, AudioRef, Character, Costume, EpisodeIndex, FileRef, MovieRef, Warning, WarningKind,
};
use ripper_format::unpack::{FileKind, UnpackRecord, UnpackedFile};

use crate::plan::Plan;

/// Unpack records of the planned bundles, keyed by bundle name (`library/<bundle>/_ripper.json`).
pub type Records = BTreeMap<String, UnpackRecord>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    #[error("scenario bundle {0} has no unpack record")]
    ScenarioNotUnpacked(String),
    #[error("scenario {scenario_id} is not in the unpack record of {bundle}")]
    ScenarioNotFound { bundle: String, scenario_id: String },
}

/// Last `/`-separated segment of a container path.
fn file_name(container: &str) -> &str {
    container.rsplit('/').next().unwrap_or(container)
}

/// File name without its last extension.
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// The `ScenarioSceneData` of `scenario_id` in an unpacked scenario bundle (container
/// `…/<ScenarioId>.asset`, compared case-insensitively).
pub fn find_scenario_file<'a>(
    record: &'a UnpackRecord,
    scenario_id: &str,
) -> Option<&'a UnpackedFile> {
    record.files.iter().find(|f| {
        f.kind == FileKind::Typetree
            && stem(file_name(&f.container)).eq_ignore_ascii_case(scenario_id)
    })
}

/// Container file names of objects the unpacker skipped (`"<container>: <reason>"`), lower-cased.
fn skipped_names(record: &UnpackRecord) -> HashSet<String> {
    record
        .skipped
        .iter()
        .filter_map(|s| {
            s.split_once(": ")
                .map(|(container, _)| file_name(container).to_lowercase())
        })
        .collect()
}

/// The clips of one bundle by lower-cased container file name (`<name>.anim`).
struct Clips<'a> {
    bundle: &'a str,
    files: HashMap<String, &'a UnpackedFile>,
    skipped: HashSet<String>,
}

impl<'a> Clips<'a> {
    fn new(bundle: &'a str, record: &'a UnpackRecord) -> Self {
        let mut files = HashMap::new();
        for file in record.files.iter().filter(|f| f.kind == FileKind::Motion) {
            // Container paths are lower-case already; the first clip of a name wins.
            files
                .entry(file_name(&file.container).to_lowercase())
                .or_insert(file);
        }
        Self {
            bundle,
            files,
            skipped: skipped_names(record),
        }
    }

    fn find(&self, key: &str) -> Option<FileRef> {
        self.files
            .get(key)
            .map(|file| FileRef::in_bundle(self.bundle, &file.path))
    }
}

struct Indexer<'r> {
    records: &'r Records,
    warnings: Vec<Warning>,
    not_unpacked: HashSet<String>,
}

impl<'r> Indexer<'r> {
    /// The record of a bundle, warning once when it is missing.
    fn record(&mut self, bundle: &str) -> Option<&'r UnpackRecord> {
        let record = self.records.get(bundle);
        if record.is_none() && self.not_unpacked.insert(bundle.to_owned()) {
            self.warnings.push(Warning::new(
                WarningKind::BundleNotUnpacked,
                format!("{bundle} has no unpack record"),
            ));
        }
        record
    }

    fn character(&mut self, planned: &crate::plan::PlannedCharacter) -> Character {
        let motion_clips = planned
            .motion_bundle
            .as_deref()
            .and_then(|b| Some(Clips::new(b, self.record(b)?)));
        let model_clips: Vec<Option<Clips<'_>>> = planned
            .costumes
            .iter()
            .map(|(_, bundle)| {
                let bundle = bundle.as_deref()?;
                Some(Clips::new(bundle, self.record(bundle)?))
            })
            .collect();

        let mut costumes: Vec<Costume> = planned
            .costumes
            .iter()
            .map(|(costume_type, bundle)| Costume {
                costume_type: costume_type.clone(),
                model_bundle: bundle.clone(),
                motions: BTreeMap::new(),
            })
            .collect();
        for name in &planned.motions {
            // `RegisterMotion`: `<name>.anim` in the model bundle first, then the motion bundle.
            let key = format!("{}.anim", name.to_lowercase());
            let mut found = false;
            for (costume, model) in costumes.iter_mut().zip(&model_clips) {
                let clip = model
                    .as_ref()
                    .and_then(|m| m.find(&key))
                    .or_else(|| motion_clips.as_ref().and_then(|m| m.find(&key)));
                if let Some(clip) = clip {
                    costume.motions.insert(name.clone(), clip);
                    found = true;
                }
            }
            if found {
                continue;
            }
            let skipped_in = model_clips
                .iter()
                .flatten()
                .chain(&motion_clips)
                .find(|c| c.skipped.contains(&key));
            if let Some(clips) = skipped_in {
                self.warnings.push(Warning::new(
                    WarningKind::FileNotFound,
                    format!(
                        "Character2dId {}: {key} was skipped when unpacking {}",
                        planned.id, clips.bundle
                    ),
                ));
            } else if motion_clips.is_some() {
                // Without the motion bundle the cause is already reported.
                self.warnings.push(Warning::new(
                    WarningKind::MotionNotFound,
                    format!(
                        "Character2dId {}: {name} is in none of the models {:?} nor {} (the game keeps the previous motion)",
                        planned.id,
                        planned.costumes.iter().map(|(c, _)| c).collect::<Vec<_>>(),
                        planned.motion_bundle.as_deref().unwrap_or_default(),
                    ),
                ));
            }
        }
        Character {
            asset_name: planned.asset_name.clone(),
            motion_bundle: planned.motion_bundle.clone(),
            costumes,
        }
    }

    fn background(&mut self, planned: &crate::plan::PlannedBackground) -> Option<FileRef> {
        let record = self.record(&planned.bundle)?;
        let pngs: Vec<&UnpackedFile> = record
            .files
            .iter()
            .filter(|f| f.kind == FileKind::Png)
            .collect();
        let file = pngs
            .iter()
            .find(|f| stem(file_name(&f.container)).eq_ignore_ascii_case(&planned.file))
            .or(match pngs.as_slice() {
                [only] => Some(only),
                _ => None,
            });
        match file {
            Some(file) => Some(FileRef::in_bundle(&planned.bundle, &file.path)),
            None => {
                self.warnings.push(Warning::new(
                    WarningKind::FileNotFound,
                    format!(
                        "background {}: no texture {:?} in {} ({} textures)",
                        planned.key,
                        planned.file,
                        planned.bundle,
                        pngs.len()
                    ),
                ));
                None
            }
        }
    }

    fn audio<F>(
        &mut self,
        bundles: &[String],
        cue: &str,
        what: &str,
        lookup: &F,
    ) -> Option<AudioRef>
    where
        F: Fn(&str, &str) -> Option<(String, Vec<String>)>,
    {
        let found = bundles
            .iter()
            .find_map(|bundle| lookup(bundle, cue).map(|hit| (bundle, hit)));
        match found {
            Some((bundle, (acb, files))) => Some(AudioRef {
                bundle: bundle.clone(),
                acb: episode::library_path(bundle, &acb),
                cue: cue.to_owned(),
                files: files
                    .iter()
                    .map(|f| episode::library_path(bundle, f))
                    .collect(),
            }),
            None => {
                // No bundle to search: already reported by the plan.
                if !bundles.is_empty() {
                    self.warnings.push(Warning::new(
                        WarningKind::CueNotFound,
                        format!("{what} {cue}: not in {bundles:?}"),
                    ));
                }
                None
            }
        }
    }
}

/// Builds the episode index.
///
/// `cue_lookup(bundle, cue)` returns the cue's `.acb` and its waveform files, both relative to
/// `library/<bundle>/` (the convention of `UnpackRecord` file paths), or `None` when no ACB of
/// the bundle has the cue. Cue names are matched exactly (some end in spaces). A bundle can hold
/// several ACBs (card voice bundles: `001001_ichika02` also has `001002_ichika02.acb`, ...): the
/// lookup should prefer the ACB whose stem is the episode's `scenarioId` (`plan.story`), then
/// any other.
pub fn build_index<F>(
    plan: &Plan,
    records: &Records,
    cue_lookup: F,
) -> Result<EpisodeIndex, IndexError>
where
    F: Fn(&str, &str) -> Option<(String, Vec<String>)>,
{
    let scenario_record = records
        .get(&plan.scenario_bundle)
        .ok_or_else(|| IndexError::ScenarioNotUnpacked(plan.scenario_bundle.clone()))?;
    let scenario_file =
        find_scenario_file(scenario_record, &plan.story.scenario_id).ok_or_else(|| {
            IndexError::ScenarioNotFound {
                bundle: plan.scenario_bundle.clone(),
                scenario_id: plan.story.scenario_id.clone(),
            }
        })?;

    let mut indexer = Indexer {
        records,
        warnings: plan.warnings.clone(),
        not_unpacked: HashSet::new(),
    };

    let characters = plan
        .characters
        .iter()
        .map(|c| (c.id, indexer.character(c)))
        .collect();

    let mut backgrounds = BTreeMap::new();
    for planned in &plan.backgrounds {
        if let Some(file) = indexer.background(planned) {
            backgrounds.insert(planned.key.clone(), file);
        }
    }

    let mut movies = BTreeMap::new();
    for (name, bundle) in &plan.movies {
        if let Some(record) = indexer.record(bundle) {
            let files = record
                .files
                .iter()
                .map(|f| episode::library_path(bundle, &f.path))
                .collect();
            movies.insert(
                name.clone(),
                MovieRef {
                    bundle: bundle.clone(),
                    files,
                },
            );
        }
    }

    let mut bgm = BTreeMap::new();
    for (name, bundle) in &plan.bgm {
        if let Some(audio) = indexer.audio(std::slice::from_ref(bundle), name, "BGM", &cue_lookup) {
            bgm.insert(name.clone(), audio);
        }
    }
    let mut se = BTreeMap::new();
    for planned in &plan.se {
        if let Some(audio) = indexer.audio(&planned.bundles, &planned.cue, "SE", &cue_lookup) {
            se.insert(planned.cue.clone(), audio);
        }
    }
    let mut voices = BTreeMap::new();
    for planned in &plan.voices {
        if let Some(audio) = indexer.audio(&planned.bundles, &planned.cue, "voice", &cue_lookup) {
            voices.insert(planned.cue.clone(), audio);
        }
    }

    let mut effects: BTreeMap<String, String> = BTreeMap::new();
    for (name, bundle) in &plan.effects {
        match effects.get(name) {
            None => {
                effects.insert(name.clone(), bundle.clone());
            }
            Some(existing) if existing != bundle => {
                effects.insert(bundle.clone(), bundle.clone());
            }
            Some(_) => {}
        }
    }

    Ok(EpisodeIndex {
        format: episode::FORMAT.into(),
        version: episode::VERSION,
        story: plan.story.clone(),
        scenario: FileRef::in_bundle(&plan.scenario_bundle, &scenario_file.path),
        bundles: plan.bundles.clone(),
        characters,
        backgrounds,
        bgm,
        se,
        voices,
        effects,
        movies,
        music_videos: plan.music_videos.clone(),
        warnings: indexer.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::tests::test_plan;
    use ripper_format::unpack;

    fn record(
        bundle: &str,
        root: &str,
        files: &[(&str, FileKind, &str)],
    ) -> (String, UnpackRecord) {
        let record = UnpackRecord {
            format: unpack::FORMAT.into(),
            version: unpack::VERSION,
            bundle: bundle.into(),
            crc: 0,
            container_root: root.into(),
            files: files
                .iter()
                .enumerate()
                .map(|(i, (path, kind, container))| UnpackedFile {
                    path: (*path).into(),
                    kind: *kind,
                    container: format!("{root}{container}"),
                    path_id: i as i64,
                })
                .collect(),
            skipped: Vec::new(),
            unresolved_bindings: Vec::new(),
        };
        (bundle.into(), record)
    }

    fn records() -> Records {
        use FileKind::*;
        let root = "assets/sekai/assetbundle/resources/startapp/";
        let mut records: Records = [
            record(
                "scenario/unitstory/school-refusal-story-chapter",
                root,
                &[
                    ("nightcode_01_00.json", Typetree, "nightcode_01_00.asset"),
                    ("nightcode_01_01.json", Typetree, "nightcode_01_01.asset"),
                ],
            ),
            record(
                "live2d/model/17kanade_normal",
                root,
                &[
                    ("17kanade_normal.moc3", Text, "17kanade_normal.moc3.bytes"),
                    (
                        "motions/w-kanade-angry01.sse-motion.json",
                        Motion,
                        "motions/w-kanade-angry01.anim",
                    ),
                ],
            ),
            record("live2d/model/17kanade_cloth001", root, &[]),
            record(
                "live2d/motion/17kanade_motion_base",
                root,
                &[
                    (
                        "motion/w-kanade-angry01.sse-motion.json",
                        Motion,
                        "motion/w-kanade-angry01.anim",
                    ),
                    (
                        "motion/w-kanade-idle01.sse-motion.json",
                        Motion,
                        "motion/w-kanade-idle01.anim",
                    ),
                    (
                        "facial/face_normal_01.sse-motion.json",
                        Motion,
                        "facial/face_normal_01.anim",
                    ),
                    ("buildmotiondata.json", Typetree, "buildmotiondata.asset"),
                ],
            ),
            record(
                "scenario/background/bg_a000000",
                root,
                &[("bg_a000000.png", Png, "bg_a000000.png")],
            ),
            record(
                "scenario/background/bg_h000104",
                root,
                &[("other.png", Png, "other.png")],
            ),
            record(
                "scenario/background/bg_e000102",
                root,
                &[
                    ("bg_e000102.png", Png, "bg_e000102.png"),
                    ("bg_e000102_night.png", Png, "bg_e000102_night.png"),
                ],
            ),
            record(
                "scenario/movie/school_refusal_opening",
                root,
                &[(
                    "school_refusal_opening.usm",
                    Text,
                    "school_refusal_opening.usm.bytes",
                )],
            ),
        ]
        .into_iter()
        .collect();
        // A clip that exists in the bundle but failed to convert.
        records
            .get_mut("live2d/motion/17kanade_motion_base")
            .unwrap()
            .skipped
            .push(format!("{root}facial/face_angry_01.anim: malformed clip"));
        records
    }

    fn cues(bundle: &str, cue: &str) -> Option<(String, Vec<String>)> {
        let hit = matches!(
            (bundle, cue),
            ("sound/scenario/bgm/bgm00000", "bgm00000")
                | ("sound/scenario/se/se_pack00001", "se00001")
                | ("sound/scenario/se/se_pack00001_b", "se00600")
                | (
                    "event_story/event_x_2023/scenario_se",
                    "se_event_x_2023_001"
                )
                | ("sound/scenario/voice/nightcode_01_01", "voice_18_01")
                | (
                    "sound/scenario/part_voice/17kanade_school_refusal",
                    "partvoice_17kanade_01"
                )
        );
        let acb = bundle.rsplit('/').next().unwrap();
        hit.then(|| (format!("{acb}.acb"), vec![format!("{acb}.audio/e0.wav")]))
    }

    fn index() -> EpisodeIndex {
        build_index(&test_plan(), &records(), cues).unwrap()
    }

    fn details(index: &EpisodeIndex, kind: WarningKind) -> Vec<&str> {
        index
            .warnings
            .iter()
            .filter(|w| w.kind == kind)
            .map(|w| w.detail.as_str())
            .collect()
    }

    #[test]
    fn indexes_the_scenario_and_backgrounds() {
        let index = index();
        assert_eq!(index.format, "ripper-episode");
        assert_eq!(
            index.scenario.path,
            "scenario/unitstory/school-refusal-story-chapter/nightcode_01_01.json"
        );
        assert_eq!(
            index.backgrounds["bg_a000000"].path,
            "scenario/background/bg_a000000/bg_a000000.png"
        );
        // The only texture of a bundle is used even when its name differs.
        assert_eq!(
            index.backgrounds["bg_h000104"].path,
            "scenario/background/bg_h000104/other.png"
        );
        assert_eq!(
            index.backgrounds["bg_e000102/bg_e000102_night"].path,
            "scenario/background/bg_e000102/bg_e000102_night.png"
        );
        assert!(!index.backgrounds.contains_key("bg_e000101"));
        assert!(
            details(&index, WarningKind::BundleNotUnpacked)
                .contains(&"scenario/background/bg_e000101 has no unpack record")
        );
        assert_eq!(
            index.movies["school_refusal_opening"].files,
            ["scenario/movie/school_refusal_opening/school_refusal_opening.usm"]
        );
        assert_eq!(index.effects["hologram"], "scenario/effect/hologram");
        assert_eq!(index.music_videos, ["5"]);
    }

    #[test]
    fn resolves_motions_model_bundle_first_per_costume() {
        let index = index();
        let kanade = &index.characters[&17];
        assert_eq!(kanade.asset_name.as_deref(), Some("17kanade"));
        let normal = &kanade.costumes[0];
        assert_eq!(normal.costume_type, "17kanade_normal");
        assert_eq!(
            normal.motions["w-kanade-angry01"].path,
            "live2d/model/17kanade_normal/motions/w-kanade-angry01.sse-motion.json"
        );
        assert_eq!(
            normal.motions["w-kanade-idle01"].bundle,
            "live2d/motion/17kanade_motion_base"
        );
        // Without that clip in its model, the other costume falls back to the motion bundle.
        let cloth = &kanade.costumes[1];
        assert_eq!(
            cloth.motions["w-kanade-angry01"].path,
            "live2d/motion/17kanade_motion_base/motion/w-kanade-angry01.sse-motion.json"
        );
        // The costume whose model is not in the manifest still resolves from the motion bundle.
        assert_eq!(kanade.costumes[2].model_bundle, None);
        assert_eq!(kanade.costumes[2].motions.len(), 3);
    }

    #[test]
    fn motion_names_match_case_insensitively() {
        let mut plan = test_plan();
        plan.characters[0].motions = vec!["W-Kanade-IDLE01".into()];
        let index = build_index(&plan, &records(), cues).unwrap();
        assert_eq!(
            index.characters[&17].costumes[0].motions["W-Kanade-IDLE01"].path,
            "live2d/motion/17kanade_motion_base/motion/w-kanade-idle01.sse-motion.json"
        );
    }

    #[test]
    fn reports_missing_and_skipped_motions() {
        let mut plan = test_plan();
        plan.characters[0].motions.push("w-kanade-missing01".into());
        let index = build_index(&plan, &records(), cues).unwrap();
        let not_found = details(&index, WarningKind::MotionNotFound);
        assert_eq!(not_found.len(), 1);
        assert!(not_found[0].contains("w-kanade-missing01"));
        assert_eq!(
            details(&index, WarningKind::FileNotFound),
            [
                "Character2dId 17: face_angry_01.anim was skipped when unpacking live2d/motion/17kanade_motion_base"
            ]
        );
        // 18 has no motion bundle: its names are not reported one by one.
        assert!(!not_found.iter().any(|d| d.contains("Character2dId 18")));
        assert!(index.characters[&18].costumes[0].motions.is_empty());
    }

    #[test]
    fn looks_audio_up_through_the_callback_in_order() {
        let index = index();
        let bgm = &index.bgm["bgm00000"];
        assert_eq!(bgm.acb, "sound/scenario/bgm/bgm00000/bgm00000.acb");
        assert_eq!(
            bgm.files,
            ["sound/scenario/bgm/bgm00000/bgm00000.audio/e0.wav"]
        );
        assert_eq!(index.se["se00001"].bundle, "sound/scenario/se/se_pack00001");
        assert_eq!(
            index.se["se00600"].bundle,
            "sound/scenario/se/se_pack00001_b"
        );
        assert_eq!(
            index.se["se_event_x_2023_001"].acb,
            "event_story/event_x_2023/scenario_se/scenario_se.acb"
        );
        assert_eq!(
            index.voices["voice_18_01"].bundle,
            "sound/scenario/voice/nightcode_01_01"
        );
        assert_eq!(
            index.voices["partvoice_17kanade_01"].bundle,
            "sound/scenario/part_voice/17kanade_school_refusal"
        );
        let cue_warnings = details(&index, WarningKind::CueNotFound);
        assert_eq!(cue_warnings.len(), 2, "{cue_warnings:?}");
        assert!(cue_warnings[0].starts_with("SE se_ev01: not in"));
        assert!(cue_warnings[1].starts_with("voice voice_op_01: not in"));
        // The plan's warnings are carried over.
        assert!(!details(&index, WarningKind::OutOfScope).is_empty());
    }

    #[test]
    fn fails_without_the_scenario() {
        let mut records = records();
        records.remove("scenario/unitstory/school-refusal-story-chapter");
        assert!(matches!(
            build_index(&test_plan(), &records, cues),
            Err(IndexError::ScenarioNotUnpacked(_))
        ));
        let mut plan = test_plan();
        plan.story.scenario_id = "nightcode_09_09".into();
        assert!(matches!(
            build_index(&plan, &self::records(), cues),
            Err(IndexError::ScenarioNotFound { .. })
        ));
    }
}
