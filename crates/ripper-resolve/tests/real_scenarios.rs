//! Runs the resolver over real `ScenarioSceneData` JSON files from a local corpus. Ignored by
//! default and skipped when the corpus is absent: no game data is committed.
//!
//! ```text
//! RIPPER_TEST_SCENARIO_DIR=…/assets/scenario \
//! RIPPER_TEST_MASTER_DIR=…/master            # optional: <table>.json of catalog::TABLES
//! RIPPER_TEST_MANIFEST=…/manifest.json       # optional: {"bundles": {name: {"dependencies": [..]}}}
//! cargo test -p ripper-resolve --test real_scenarios -- --ignored --nocapture
//! ```
//!
//! Without `RIPPER_TEST_SCENARIO_DIR` it looks for `pjskChatGenerator/assets/scenario` next to
//! the workspace. With masterdata and a manifest it also plans every episode found and requires
//! that the Live2D rules leave no model or motion bundle unresolved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ripper_format::episode::WarningKind;
use ripper_resolve::catalog::{Catalog, Masterdata, TABLES};
use ripper_resolve::plan::{PlanOptions, plan, scenario_bundle_candidates};
use ripper_resolve::references::extract;

fn scenario_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("RIPPER_TEST_SCENARIO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pjskChatGenerator/assets/scenario")
        });
    dir.is_dir().then_some(dir)
}

fn json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            json_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

fn masterdata() -> Option<Masterdata> {
    let dir = PathBuf::from(std::env::var_os("RIPPER_TEST_MASTER_DIR")?);
    let mut md = Masterdata::default();
    for table in TABLES {
        let bytes = std::fs::read(dir.join(format!("{table}.json"))).unwrap();
        md.load_table(table, &bytes).unwrap();
    }
    Some(md)
}

fn manifest() -> Option<BTreeMap<String, Vec<String>>> {
    let path = std::env::var_os("RIPPER_TEST_MANIFEST")?;
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let bundles = json["bundles"]
        .as_object()
        .expect("manifest has no bundles map");
    Some(
        bundles
            .iter()
            .map(|(name, entry)| {
                let deps = entry["dependencies"]
                    .as_array()
                    .map(|d| {
                        d.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                (name.clone(), deps)
            })
            .collect(),
    )
}

#[test]
#[ignore = "needs a local scenario corpus"]
fn resolves_real_scenarios() {
    let Some(dir) = scenario_dir() else {
        eprintln!("no scenario corpus; skipped");
        return;
    };
    let mut files = Vec::new();
    json_files(&dir, &mut files);
    assert!(!files.is_empty(), "no JSON under {}", dir.display());

    let context = masterdata().zip(manifest());
    let catalog = context.as_ref().map(|(md, _)| Catalog::new(md));
    let character2ds = context.as_ref().map(|(md, _)| md.character2d_index());
    let options = PlanOptions::default();
    let mut planned = 0;

    for path in &files {
        let scenario: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let refs = extract(&scenario).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // The asset name is the masterdata scenarioId; the ScenarioId field may differ.
        let stem = path.file_stem().unwrap().to_string_lossy();
        if refs.scenario_id != stem {
            eprintln!(
                "  ScenarioId field {:?} differs from the asset name",
                refs.scenario_id
            );
        }
        // `<corpus>/<dir>/<scenarioId>.json` mirrors `scenario/<dir>`.
        let corpus_bundle = path
            .parent()
            .and_then(|p| p.strip_prefix(&dir).ok())
            .map(|rel| format!("scenario/{}", rel.to_string_lossy().replace('\\', "/")));
        let motions: usize = refs.characters.iter().map(|c| c.motions.len()).sum();
        eprintln!(
            "{stem}: {} characters / {motions} motion names, {} backgrounds, {} movies, {} voices, \
             {} bgm, {} se, {} effects, {} mv, {} warnings",
            refs.characters.len(),
            refs.backgrounds.len(),
            refs.movies.len(),
            refs.voices.len(),
            refs.bgm.len(),
            refs.se.len(),
            refs.effects.len(),
            refs.music_videos.len(),
            refs.warnings.len(),
        );
        for warning in &refs.warnings {
            eprintln!("  {:?}: {}", warning.kind, warning.detail);
        }
        // Real scenarios always name a costume for every character.
        for character in &refs.characters {
            assert!(!character.costumes.is_empty(), "{stem}: {character:?}");
        }

        let (Some(catalog), Some(character2ds), Some((_, manifest))) =
            (&catalog, &character2ds, &context)
        else {
            continue;
        };
        let Some(episode) = catalog
            .episodes
            .iter()
            .find(|e| e.story.scenario_id == stem)
        else {
            eprintln!("  not in masterdata; not planned");
            continue;
        };
        let candidates = scenario_bundle_candidates(episode, manifest);
        let bundle = candidates
            .iter()
            .find(|b| Some(**b) == corpus_bundle.as_deref())
            .or(candidates.first())
            .expect("scenario bundle in manifest");
        let plan = plan(episode, bundle, &refs, manifest, character2ds, &options);
        planned += 1;
        eprintln!(
            "  {}: {} bundles",
            episode.story.selector,
            plan.bundles.len()
        );
        for warning in &plan.warnings[refs.warnings.len()..] {
            eprintln!("  {:?}: {}", warning.kind, warning.detail);
        }
        for kind in [
            WarningKind::UnresolvedMotionBundle,
            WarningKind::ModelBundleMissing,
            WarningKind::MasterManifestMismatch,
        ] {
            assert!(
                !plan.warnings.iter().any(|w| w.kind == kind),
                "{stem}: {kind:?} in {:?}",
                plan.warnings
            );
        }
    }
    eprintln!("{} scenarios extracted, {planned} planned", files.len());
}
