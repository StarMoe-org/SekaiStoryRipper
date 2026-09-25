//! `ripper plan` / `ripper rip`: resolve story episodes to bundles, and export them completely.
//!
//! plan: masterdata + manifest → episodes → scenario (fetched and unpacked) → references → plan.
//! rip: plan, fetch + unpack every planned bundle, then write `<out>/episodes/<type>/<key>/<no>.json`
//! (`ripper-episode` v1) and `<out>/ripper.lock.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use ripper_cdn::{BundleEntry, Manifest};
use ripper_convert::unpack::read_record;
use ripper_format::episode::{EpisodeIndex, Warning, WarningKind};
use ripper_resolve::catalog::{Catalog, Character2ds, Episode, Masterdata};
use ripper_resolve::index::{Records, build_index, find_scenario_file};
use ripper_resolve::plan::{
    CueIndex, Plan, PlanOptions, plan, scenario_bundle_candidates, scenario_missing,
};
use ripper_resolve::selector::Selector;
use serde::Serialize;

use crate::audio_lookup::AudioLookup;
use crate::config::Config;
use crate::fetch_cmd::load_manifest;
use crate::masterdata_cmd;
use crate::unpack_cmd::unpack_entries;

pub struct Args {
    pub selectors: Vec<String>,
    pub asset_version: Option<String>,
    /// Write every plan (plan) / a summary (rip) as JSON.
    pub report: Option<PathBuf>,
    /// Fail when any episode has warnings.
    pub strict: bool,
    pub force: bool,
    pub keep_astc: bool,
}

/// Bundle name → dependencies, the resolver's view of the manifest.
fn manifest_view(manifest: &Manifest) -> BTreeMap<String, Vec<String>> {
    manifest
        .bundles
        .iter()
        .map(|(name, entry)| (name.clone(), entry.dependencies.clone()))
        .collect()
}

fn library(config: &Config) -> PathBuf {
    config.paths.out.join("library")
}

fn bundle_dir(library: &Path, bundle: &str) -> PathBuf {
    let mut dir = library.to_path_buf();
    dir.extend(bundle.split('/'));
    dir
}

fn entries_for<'a>(
    manifest: &Manifest,
    names: impl IntoIterator<Item = &'a String>,
) -> Vec<BundleEntry> {
    names
        .into_iter()
        .filter_map(|name| manifest.bundles.get(name).cloned())
        .collect()
}

async fn load_masterdata(config: &Config) -> Result<Masterdata> {
    let mut masterdata = Masterdata::default();
    for table in ripper_resolve::catalog::TABLES {
        let json = masterdata_cmd::table(config, table).await?;
        masterdata
            .load_table(table, &serde_json::to_vec(&json)?)
            .with_context(|| format!("masterdata {table}"))?;
    }
    Ok(masterdata)
}

fn select<'a>(catalog: &'a Catalog, selectors: &[String]) -> Result<Vec<&'a Episode>> {
    if selectors.is_empty() {
        bail!(
            "name at least one selector, e.g. unit:school-refusal-story-chapter/1, event:120, card:1, special:2, all"
        );
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for text in selectors {
        let selector = Selector::from_str(text).map_err(|e| anyhow::anyhow!("{text}: {e}"))?;
        let chosen = catalog.select(&selector);
        if chosen.is_empty() {
            bail!("{text}: matches no episode");
        }
        for episode in chosen {
            if seen.insert(episode.story.selector.clone()) {
                out.push(episode);
            }
        }
    }
    Ok(out)
}

struct Planned<'a> {
    episode: &'a Episode,
    plan: Result<Plan, Warning>,
}

/// Plans every selected episode; fetches and unpacks their scenario bundle candidates on the way.
async fn plan_episodes<'a>(
    config: &Config,
    manifest: &Manifest,
    episodes: &[&'a Episode],
    character2ds: &Character2ds,
    options: &PlanOptions,
) -> Result<Vec<Planned<'a>>> {
    let view = manifest_view(manifest);
    let candidates: BTreeSet<String> = episodes
        .iter()
        .flat_map(|e| {
            scenario_bundle_candidates(e, &view)
                .into_iter()
                .map(str::to_owned)
        })
        .collect();
    let summary = unpack_entries(
        config,
        entries_for(manifest, &candidates),
        false,
        false,
        false,
    )
    .await?;
    for (bundle, reason) in &summary.failed {
        eprintln!("warning: scenario bundle {bundle} failed to unpack: {reason}");
    }
    let library = library(config);
    let mut planned = Vec::with_capacity(episodes.len());
    for &episode in episodes {
        let mut found = None;
        for bundle in scenario_bundle_candidates(episode, &view) {
            let dir = bundle_dir(&library, bundle);
            let Some(record) = read_record(&dir) else {
                continue;
            };
            if let Some(file) = find_scenario_file(&record, &episode.story.scenario_id) {
                found = Some((bundle.to_owned(), dir.join(&file.path)));
                break;
            }
        }
        let plan = match found {
            None => Err(scenario_missing(episode)),
            Some((bundle, path)) => {
                let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("{}", path.display()))?;
                match ripper_resolve::references::extract(&json) {
                    Ok(refs) => Ok(plan(episode, &bundle, &refs, &view, character2ds, options)),
                    Err(error) => Err(Warning::new(
                        WarningKind::FileNotFound,
                        format!("{}: {error}", path.display()),
                    )),
                }
            }
        };
        planned.push(Planned { episode, plan });
    }
    Ok(planned)
}

fn warning_counts<'a>(warnings: impl IntoIterator<Item = &'a Warning>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for warning in warnings {
        let kind = serde_json::to_value(warning.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        *counts.entry(kind).or_default() += 1;
    }
    counts
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanReport<'a> {
    asset_version: String,
    episodes: usize,
    missing_scenarios: Vec<&'a Warning>,
    warning_counts: BTreeMap<String, usize>,
    plans: Vec<&'a Plan>,
}

async fn prepare(config: &Config, args: &Args) -> Result<(String, Manifest, Masterdata)> {
    let (version, manifest) = load_manifest(config, args.asset_version.as_deref())?;
    let masterdata = load_masterdata(config).await?;
    Ok((version, manifest, masterdata))
}

pub async fn run_plan(config: &Config, args: Args) -> Result<()> {
    let (version, manifest, masterdata) = prepare(config, &args).await?;
    let catalog = Catalog::new(&masterdata);
    let episodes = select(&catalog, &args.selectors)?;
    eprintln!("{} episodes selected", episodes.len());
    let character2ds = masterdata.character2d_index();
    let planned = plan_episodes(
        config,
        &manifest,
        &episodes,
        &character2ds,
        &PlanOptions::default(),
    )
    .await?;

    let plans: Vec<&Plan> = planned
        .iter()
        .filter_map(|p| p.plan.as_ref().ok())
        .collect();
    let missing: Vec<&Warning> = planned
        .iter()
        .filter_map(|p| p.plan.as_ref().err())
        .collect();
    let bundles: BTreeSet<&String> = plans.iter().flat_map(|p| &p.bundles).collect();
    let bytes: u64 = bundles
        .iter()
        .filter_map(|b| manifest.bundles.get(*b))
        .map(|e| e.file_size)
        .sum();
    let counts = warning_counts(
        plans
            .iter()
            .flat_map(|p| &p.warnings)
            .chain(missing.iter().copied()),
    );
    for p in &planned {
        match &p.plan {
            Ok(plan) if !plan.warnings.is_empty() || episodes.len() <= 20 => {
                eprintln!(
                    "{}: {} bundles, {} warnings",
                    p.episode.story.selector,
                    plan.bundles.len(),
                    plan.warnings.len()
                );
            }
            Ok(_) => {}
            Err(warning) => eprintln!(
                "{}: NO SCENARIO: {}",
                p.episode.story.selector, warning.detail
            ),
        }
    }
    println!(
        "{}{version}: {} episodes planned ({} without scenario), {} distinct bundles, {:.2} GB to download",
        config.cdn.platform,
        plans.len(),
        missing.len(),
        bundles.len(),
        bytes as f64 / 1e9
    );
    println!("warnings: {}", serde_json::to_string(&counts)?);
    if let Some(path) = &args.report {
        let report = PlanReport {
            asset_version: version,
            episodes: planned.len(),
            missing_scenarios: missing,
            warning_counts: counts.clone(),
            plans,
        };
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        println!("report -> {}", path.display());
    }
    if args.strict && !counts.is_empty() {
        bail!("--strict: episodes have warnings");
    }
    Ok(())
}

/// Every part-voice bundle of the manifest (the fallback index for `partvoice_*` cues).
fn part_voice_bundles(manifest: &Manifest) -> BTreeSet<String> {
    manifest
        .bundles
        .keys()
        .filter(|n| {
            n.starts_with("sound/scenario/part_voice/")
                || n.starts_with("sound/scenario/voice/part_voice_")
        })
        .cloned()
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Lock {
    tool_version: &'static str,
    formats: BTreeMap<&'static str, u32>,
    region: String,
    app_version: String,
    asset_version: String,
    unity_version: String,
    masterdata: serde_json::Value,
    episodes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RipReport {
    asset_version: String,
    episodes: BTreeMap<String, RipEpisode>,
    warning_counts: BTreeMap<String, usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RipEpisode {
    index: Option<String>,
    bundles: usize,
    warnings: Vec<Warning>,
}

pub async fn run_rip(config: &Config, args: Args) -> Result<()> {
    let (version, manifest, masterdata) = prepare(config, &args).await?;
    let catalog = Catalog::new(&masterdata);
    let episodes = select(&catalog, &args.selectors)?;
    eprintln!("{} episodes selected", episodes.len());
    let character2ds = masterdata.character2d_index();
    let library = library(config);
    let lookup = AudioLookup::new(&library);

    // First pass without the part-voice fallback, to see whether it is needed.
    let first = plan_episodes(
        config,
        &manifest,
        &episodes,
        &character2ds,
        &PlanOptions::default(),
    )
    .await?;
    let needs_part_voice = first
        .iter()
        .filter_map(|p| p.plan.as_ref().ok())
        .any(|p| p.voices.iter().any(|v| v.cue.starts_with("partvoice")));
    let mut options = PlanOptions::default();
    if needs_part_voice {
        let bundles = part_voice_bundles(&manifest);
        eprintln!("indexing {} part-voice bundles", bundles.len());
        let summary = unpack_entries(
            config,
            entries_for(&manifest, &bundles),
            args.force,
            false,
            false,
        )
        .await?;
        for (bundle, reason) in &summary.failed {
            eprintln!("warning: {bundle}: {reason}");
        }
        let mut index = CueIndex::new();
        for bundle in &bundles {
            for cue in lookup.cue_names(bundle) {
                index.entry(cue).or_default().push(bundle.clone());
            }
        }
        options.voice_cue_index = index;
    }
    let planned = if needs_part_voice {
        plan_episodes(config, &manifest, &episodes, &character2ds, &options).await?
    } else {
        first
    };

    let all: BTreeSet<String> = planned
        .iter()
        .filter_map(|p| p.plan.as_ref().ok())
        .flat_map(|p| p.bundles.iter().cloned())
        .collect();
    let bytes: u64 = all
        .iter()
        .filter_map(|b| manifest.bundles.get(b))
        .map(|e| e.file_size)
        .sum();
    eprintln!(
        "{} bundles, {:.2} GB to fetch and unpack",
        all.len(),
        bytes as f64 / 1e9
    );
    let summary = unpack_entries(
        config,
        entries_for(&manifest, &all),
        args.force,
        args.keep_astc,
        false,
    )
    .await?;
    eprintln!(
        "{} unpacked, {} up to date, {} failed",
        summary.unpacked,
        summary.up_to_date,
        summary.failed.len()
    );

    let mut report = RipReport {
        asset_version: version.clone(),
        episodes: BTreeMap::new(),
        warning_counts: BTreeMap::new(),
    };
    let mut written = Vec::new();
    for p in &planned {
        let plan = match &p.plan {
            Ok(plan) => plan,
            Err(warning) => {
                report.episodes.insert(
                    p.episode.story.selector.clone(),
                    RipEpisode {
                        index: None,
                        bundles: 0,
                        warnings: vec![warning.clone()],
                    },
                );
                continue;
            }
        };
        let records: Records = plan
            .bundles
            .iter()
            .filter_map(|b| Some((b.clone(), read_record(&bundle_dir(&library, b))?)))
            .collect();
        // build_index wants paths relative to library/<bundle>/; AudioLookup gives library-relative ones.
        let cue_lookup = |bundle: &str, cue: &str| {
            let (acb, files) = lookup.cue(bundle, cue)?;
            let prefix = format!("{bundle}/");
            let strip = |p: String| p.strip_prefix(&prefix).map(str::to_owned).unwrap_or(p);
            Some((strip(acb), files.into_iter().map(strip).collect()))
        };
        let index: EpisodeIndex = match build_index(plan, &records, cue_lookup) {
            Ok(index) => index,
            Err(error) => {
                let warning = Warning::new(WarningKind::FileNotFound, error.to_string());
                report.episodes.insert(
                    plan.story.selector.clone(),
                    RipEpisode {
                        index: None,
                        bundles: plan.bundles.len(),
                        warnings: vec![warning],
                    },
                );
                continue;
            }
        };
        let relative = format!(
            "episodes/{}/{}/{}.json",
            serde_json::to_value(plan.story.story_type)?
                .as_str()
                .unwrap_or("unknown"),
            p.episode.story_key,
            plan.story.episode_no
        );
        ripper_format::path::check_relative(&relative).map_err(anyhow::Error::msg)?;
        let path = config.paths.out.join(&relative);
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, serde_json::to_vec_pretty(&index)?)?;
        written.push(plan.story.selector.clone());
        report.episodes.insert(
            plan.story.selector.clone(),
            RipEpisode {
                index: Some(relative),
                bundles: index.bundles.len(),
                warnings: index.warnings.clone(),
            },
        );
    }
    report.warning_counts = warning_counts(report.episodes.values().flat_map(|e| &e.warnings));

    let lock = Lock {
        tool_version: env!("CARGO_PKG_VERSION"),
        formats: [
            (
                ripper_format::motion::FORMAT,
                ripper_format::motion::VERSION,
            ),
            (
                ripper_format::unpack::FORMAT,
                ripper_format::unpack::VERSION,
            ),
            (ripper_format::audio::FORMAT, ripper_format::audio::VERSION),
            (
                ripper_format::episode::FORMAT,
                ripper_format::episode::VERSION,
            ),
        ]
        .into(),
        region: config.cdn.region.to_string(),
        app_version: config.cdn.app_version.clone(),
        asset_version: version,
        unity_version: config.unity.version.clone(),
        masterdata: std::fs::read(masterdata_cmd::dir(config).join("meta.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::Value::Null),
        episodes: written.clone(),
    };
    std::fs::write(
        config.paths.out.join("ripper.lock.json"),
        serde_json::to_vec_pretty(&lock)?,
    )?;

    for (selector, episode) in &report.episodes {
        if !episode.warnings.is_empty() || report.episodes.len() <= 20 {
            eprintln!(
                "{selector}: {} bundles, {} warnings",
                episode.bundles,
                episode.warnings.len()
            );
            for warning in episode.warnings.iter().take(3) {
                eprintln!("    {:?}: {}", warning.kind, warning.detail);
            }
        }
    }
    let destination = match &config.remote {
        Some(remote) => format!("{}/episodes (after publishing)", remote.location()),
        None => config.paths.out.join("episodes").display().to_string(),
    };
    println!(
        "{} of {} episodes exported -> {destination}",
        written.len(),
        planned.len(),
    );
    println!(
        "warnings: {}",
        serde_json::to_string(&report.warning_counts)?
    );
    if let Some(path) = &args.report {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if !summary.failed.is_empty() {
        bail!("{} bundle(s) failed to unpack", summary.failed.len());
    }
    if args.strict && !report.warning_counts.is_empty() {
        bail!("--strict: episodes have warnings");
    }
    Ok(())
}
