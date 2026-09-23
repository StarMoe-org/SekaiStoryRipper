//! `ripper spike`: the M0 technology check (docs/plan.md §2.3).
//!
//! Unpacks every bundle under the cache with the production readers/converters and writes plain
//! files that `tools/oracle/m0/compare.py` checks against UnityPy. Output is game-derived data and
//! must stay out of the repository (D5).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use ripper_convert::{audio, moc3, motion, movie, texture};
use ripper_format::motion::Source;
use ripper_unity::{BundleSource, ObjectInfo, UnityRsBundle, class_id};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleReport {
    open_ms: u128,
    classes: BTreeMap<i32, usize>,
    typetree_ok: usize,
    typetree_errors: Vec<String>,
    motions: usize,
    motion_errors: Vec<String>,
    textures: usize,
    texture_errors: Vec<String>,
    moc3: Vec<String>,
    acb_cues: usize,
    acb_errors: Vec<String>,
    usm_streams: Vec<String>,
    usm_errors: Vec<String>,
    content_crc32: Option<u32>,
    manifest_crc: Option<u32>,
    crc_matches: Option<bool>,
    error: Option<String>,
}

fn bundle_names(cache: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path
                .extension()
                .is_none_or(|ext| ext != "json" && ext != "tmp")
            {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(relative);
            }
        }
        Ok(())
    }
    let mut names = Vec::new();
    walk(cache, cache, &mut names)?;
    names.sort();
    Ok(names)
}

fn write(path: PathBuf, bytes: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<()> {
    write(path, serde_json::to_vec_pretty(value)?)
}

/// File-name-safe rendering of an object or cue name.
fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn run(cache: &Path, out: &Path, unity_version: &str) -> Result<()> {
    let manifest_crc: BTreeMap<String, u32> = fs::read(cache.join("samples.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, Value>>(&bytes).ok())
        .map(|samples| {
            samples
                .into_iter()
                .filter_map(|(name, v)| Some((name, v.get("manifest_crc")?.as_u64()? as u32)))
                .collect()
        })
        .unwrap_or_default();

    let mut bundles = Vec::new();
    let mut reports = BTreeMap::new();
    for name in bundle_names(cache)? {
        let bytes = fs::read(cache.join(&name))?;
        let started = Instant::now();
        match UnityRsBundle::open(&name, bytes, unity_version) {
            Ok(bundle) => {
                let report = BundleReport {
                    open_ms: started.elapsed().as_millis(),
                    ..BundleReport::default()
                };
                reports.insert(name.clone(), report);
                bundles.push((name, bundle));
            }
            Err(error) => {
                eprintln!("{name}: open failed: {error}");
                reports.insert(
                    name,
                    BundleReport {
                        error: Some(error.to_string()),
                        ..BundleReport::default()
                    },
                );
            }
        }
    }

    // Binding names come from every model in the cache (D12: union over a character's models).
    let mut names = motion::BindingNames::default();
    for (bundle_name, bundle) in &bundles {
        for object in bundle.objects() {
            if object.class_id != class_id::TEXT_ASSET
                || !object.name.as_deref().is_some_and(|n| n.ends_with(".moc3"))
            {
                continue;
            }
            let ids = moc3::read_moc3_ids(&bundle.text_asset(object.id)?)?;
            names.add_model(&ids.parameters, &ids.parts);
            write_json(out.join(bundle_name).join("params.json"), &ids)?;
            reports
                .get_mut(bundle_name)
                .unwrap()
                .moc3
                .push(object.name.clone().unwrap_or_default());
        }
    }

    for (bundle_name, bundle) in &bundles {
        let report = reports.get_mut(bundle_name).unwrap();
        let dir = out.join(bundle_name);
        export_bundle(bundle_name, bundle, &names, &dir, report)?;
        report.manifest_crc = manifest_crc.get(bundle_name).copied();
        match bundle.content_crc32() {
            Ok(crc) => {
                report.content_crc32 = Some(crc);
                report.crc_matches = report.manifest_crc.map(|expected| expected == crc);
            }
            Err(error) => report.error = Some(format!("crc: {error}")),
        }
        println!(
            "{bundle_name}: {} objects, typetree {}/{}, {} motions, {} textures, {} cues, crc {}",
            report.classes.values().sum::<usize>(),
            report.typetree_ok,
            report.typetree_ok + report.typetree_errors.len(),
            report.motions,
            report.textures,
            report.acb_cues,
            match report.crc_matches {
                Some(true) => "ok",
                Some(false) => "MISMATCH",
                None => "n/a",
            }
        );
    }
    write_json(out.join("spike-summary.json"), &reports)?;
    Ok(())
}

fn export_bundle(
    bundle_name: &str,
    bundle: &UnityRsBundle,
    names: &motion::BindingNames,
    dir: &Path,
    report: &mut BundleReport,
) -> Result<()> {
    let objects = bundle.objects();
    write_json(
        dir.join("objects.json"),
        &objects
            .iter()
            .map(|o| json!({"fileIndex": o.id.file_index, "pathId": o.id.path_id, "classId": o.class_id, "name": o.name, "container": o.container}))
            .collect::<Vec<_>>(),
    )?;

    let mut typetrees: BTreeMap<i64, Value> = BTreeMap::new();
    for object in &objects {
        *report.classes.entry(object.class_id).or_default() += 1;
        match bundle.typetree_json(object.id) {
            Ok(tree) => {
                report.typetree_ok += 1;
                typetrees.insert(object.id.path_id, tree);
            }
            Err(error) => report
                .typetree_errors
                .push(format!("{} {:?}: {error}", object.class_id, object.name)),
        }
    }

    // Motion metadata objects share the clip's name (`<name>.asset` next to `<name>.anim`).
    let metas: BTreeMap<&str, &Value> = objects
        .iter()
        .filter(|o| o.class_id == class_id::MONO_BEHAVIOUR)
        .filter_map(|o| {
            let tree = typetrees.get(&o.id.path_id)?;
            tree.get("FadeInTime")?;
            Some((tree.get("m_Name")?.as_str()?, tree))
        })
        .collect();

    for object in &objects {
        match object.class_id {
            class_id::MONO_BEHAVIOUR => {
                if let Some(tree) = typetrees.get(&object.id.path_id) {
                    write_json(
                        dir.join("mono").join(format!("{}.json", object.id.path_id)),
                        tree,
                    )?;
                }
            }
            class_id::ANIMATION_CLIP => {
                export_clip(bundle_name, object, &typetrees, &metas, names, dir, report)?
            }
            class_id::TEXTURE_2D => match bundle
                .texture_rgba(object.id)
                .map_err(anyhow::Error::from)
                .and_then(|image| Ok(texture::encode_png(&image)?))
            {
                Ok(png) => {
                    report.textures += 1;
                    let name = safe(object.name.as_deref().unwrap_or("texture"));
                    write(
                        dir.join("texture")
                            .join(format!("{name}__{}.png", object.id.path_id)),
                        png,
                    )?;
                }
                Err(error) => report
                    .texture_errors
                    .push(format!("{:?}: {error}", object.name)),
            },
            class_id::TEXT_ASSET => export_text_asset(bundle, object, dir, report)?,
            _ => {}
        }
    }

    for tree in typetrees
        .values()
        .filter(|tree| tree.get("movieBundleDatas").is_some())
    {
        export_movie(bundle_name, bundle, &objects, tree, dir, report)?;
    }
    Ok(())
}

fn export_movie(
    bundle_name: &str,
    bundle: &UnityRsBundle,
    objects: &[ObjectInfo],
    build_data: &Value,
    dir: &Path,
    report: &mut BundleReport,
) -> Result<()> {
    // Parts are addressed by their container file name (`<name>-NNN.usm.bytes`).
    let part = |file: &str| {
        let file = file.to_lowercase();
        let object = objects.iter().find(|o| {
            o.container
                .as_deref()
                .is_some_and(|c| c.ends_with(&format!("/{file}")))
        })?;
        bundle.text_asset(object.id).ok()
    };
    let stem = bundle_name.rsplit('/').next().unwrap_or(bundle_name);
    let streams =
        movie::assemble_usm(build_data, part).and_then(|usm| movie::demux_usm(&usm, stem));
    match streams {
        Ok(streams) => {
            for stream in streams {
                report.usm_streams.push(format!(
                    "{}.{} ({} bytes)",
                    stream.name,
                    stream.extension,
                    stream.data.len()
                ));
                write(
                    dir.join("movie")
                        .join(format!("{}.{}", safe(&stream.name), stream.extension)),
                    &stream.data,
                )?;
            }
        }
        Err(error) => report.usm_errors.push(error.to_string()),
    }
    Ok(())
}

fn export_clip(
    bundle_name: &str,
    object: &ObjectInfo,
    typetrees: &BTreeMap<i64, Value>,
    metas: &BTreeMap<&str, &Value>,
    names: &motion::BindingNames,
    dir: &Path,
    report: &mut BundleReport,
) -> Result<()> {
    let Some(clip) = typetrees.get(&object.id.path_id) else {
        return Ok(());
    };
    let source = Source {
        bundle: bundle_name.to_owned(),
        path_id: object.id.path_id,
        container: object.container.clone(),
    };
    let meta = object
        .name
        .as_deref()
        .and_then(|name| metas.get(name).copied());
    match motion::clip_to_motion(clip, meta, names, source) {
        Ok(motion) => {
            report.motions += 1;
            write_json(
                dir.join("motion")
                    .join(format!("{}.sse-motion.json", safe(&motion.name))),
                &motion,
            )?;
        }
        Err(error) => report
            .motion_errors
            .push(format!("{:?}: {error}", object.name)),
    }
    Ok(())
}

fn export_text_asset(
    bundle: &UnityRsBundle,
    object: &ObjectInfo,
    dir: &Path,
    report: &mut BundleReport,
) -> Result<()> {
    let bytes = bundle.text_asset(object.id)?;
    let name = object
        .name
        .clone()
        .unwrap_or_else(|| object.id.path_id.to_string());
    write(dir.join("text").join(safe(&name)), &bytes)?;

    if bytes.starts_with(b"@UTF") {
        let audio_dir = dir.join("audio").join(safe(&name));
        match audio::decode_acb(&bytes) {
            Ok(cues) => {
                report.acb_cues += cues.len();
                let mut index = Vec::new();
                for cue in cues {
                    let file = format!("{}.{}", safe(&cue.name), cue.extension);
                    index.push(json!({"name": cue.name, "cueId": cue.cue_id, "file": file, "hca": cue.hca}));
                    write(audio_dir.join(&file), &cue.wav)?;
                    if let Some(hca) = &cue.hca_bytes {
                        write(
                            audio_dir
                                .join("hca")
                                .join(format!("{}.hca", safe(&cue.name))),
                            hca,
                        )?;
                    }
                }
                write_json(audio_dir.join("cues.json"), &index)?;
            }
            Err(error) => report.acb_errors.push(format!("{name}: {error}")),
        }
        match audio::acb_tables(&bytes) {
            Ok(tables) => write_json(audio_dir.join("tables.json"), &tables)?,
            Err(error) => report.acb_errors.push(format!("{name} tables: {error}")),
        }
    }
    Ok(())
}
