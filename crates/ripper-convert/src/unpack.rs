//! Unpacks one bundle into `library/<bundleName>/` (format `ripper-unpack` v1).
//!
//! Every `m_Container` main asset becomes one file at its container path relative to the bundle's
//! container root, so relative references between assets keep working:
//!
//! | class | file |
//! |---|---|
//! | TextAsset | raw bytes, `.bytes` suffix dropped (`x.moc3.bytes` → `x.moc3`, `x.acb.bytes` → `x.acb`) |
//! | Texture2D | RGBA PNG |
//! | AnimationClip | `sse-motion` JSON (`x.anim` → `x.sse-motion.json`), with the fade of its `x.asset` metadata |
//! | anything else | embedded typetree as JSON (`x.asset` → `x.json`, `x.prefab` → `x.prefab.json`) |
//!
//! `Live2DBuildMotionMetaData` objects are folded into their clip and not written separately.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use ripper_format::motion::Source;
use ripper_format::unpack::{self, FileKind, UnpackRecord, UnpackedFile};
use ripper_unity::{BundleSource, ContainerEntry, ObjectInfo, class_id};
use serde_json::Value;

use crate::moc3::{MocIds, read_moc3_ids};
use crate::motion::{BindingNames, clip_to_motion};
use crate::texture::encode_png;
use crate::{ConvertError, Result, malformed};

/// Longest common directory of the container paths (with a trailing `/`), or "" when none.
fn container_root(assets: &[ContainerEntry]) -> String {
    let mut iter = assets
        .iter()
        .map(|a| a.path.rsplit_once('/').map_or("", |(dir, _)| dir));
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut common: Vec<&str> = first.split('/').collect();
    for dir in iter {
        let shared = common
            .iter()
            .zip(dir.split('/'))
            .take_while(|(a, b)| *a == b)
            .count();
        common.truncate(shared);
    }
    if common.is_empty() || common == [""] {
        String::new()
    } else {
        format!("{}/", common.join("/"))
    }
}

fn with_suffix(relative: &str, strip: &str, add: &str) -> String {
    match relative.strip_suffix(strip) {
        Some(stem) => format!("{stem}{add}"),
        None => format!("{relative}{add}"),
    }
}

/// Parameter/part ids of every moc3 TextAsset in the bundle (model bundles have one).
pub fn moc3_ids<B: BundleSource>(bundle: &B) -> Result<Vec<MocIds>> {
    bundle
        .objects()
        .iter()
        .filter(|o| {
            o.class_id == class_id::TEXT_ASSET
                && o.name.as_deref().is_some_and(|n| n.ends_with(".moc3"))
        })
        .map(|o| read_moc3_ids(&bundle.text_asset(o.id)?))
        .collect()
}

fn write_file(dir: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    let mut path = dir.to_path_buf();
    path.extend(relative.split('/'));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| malformed("write", format!("{}: {e}", parent.display())))?;
    }
    fs::write(&path, bytes).map_err(|e| malformed("write", format!("{}: {e}", path.display())))
}

/// Knobs for [`unpack_bundle`].
#[derive(Debug, Clone, Default)]
pub struct UnpackOptions {
    /// ffmpeg executable for movie ADX → WAV. `None` keeps the `.adx` only (with a note).
    pub ffmpeg: Option<std::path::PathBuf>,
}

/// One output file before it is written: `(relative path, kind, bytes)`.
type Output = (String, FileKind, Vec<u8>);

fn json_bytes(value: &impl serde::Serialize, what: &'static str) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|e| malformed(what, e.to_string()))
}

/// Accepts outputs after checking they are portable and do not collide case-insensitively.
struct Writer<'a> {
    dir: &'a Path,
    taken: HashSet<String>,
    record: UnpackRecord,
}

impl Writer<'_> {
    fn emit(&mut self, container: &str, path_id: i64, (path, kind, bytes): Output) -> Result<()> {
        if let Err(problem) = ripper_format::path::check_relative(&path) {
            self.record.skipped.push(format!("{container}: {problem}"));
            return Ok(());
        }
        // macOS and Windows file systems are case-insensitive by default.
        if !self.taken.insert(path.to_lowercase()) {
            self.record.skipped.push(format!(
                "{container}: {path} collides (case-insensitively) with another file"
            ));
            return Ok(());
        }
        write_file(self.dir, &path, &bytes)?;
        self.record.files.push(UnpackedFile {
            path,
            kind,
            container: container.to_owned(),
            path_id,
        });
        Ok(())
    }
}

/// Raw ACB plus its `ripper-acb` index, UTF tables and waveforms.
fn acb_outputs(relative: &str, bytes: Vec<u8>) -> Result<Vec<Output>> {
    let (dir, file) = relative
        .rsplit_once('/')
        .map_or(("", relative), |(d, f)| (d, f));
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let stem = file.strip_suffix(".acb").unwrap_or(file);
    let export = crate::audio::export_acb(file, &bytes)?;
    let mut outputs = vec![
        (relative.to_owned(), FileKind::Text, bytes),
        (
            format!("{prefix}{stem}.cues.json"),
            FileKind::AcbIndex,
            json_bytes(&export.index, "acb index")?,
        ),
        (
            format!("{prefix}{stem}.tables.json"),
            FileKind::AcbTables,
            json_bytes(&export.tables, "acb tables")?,
        ),
    ];
    outputs.extend(
        export
            .files
            .into_iter()
            .map(|(path, bytes)| (format!("{prefix}{path}"), FileKind::Audio, bytes)),
    );
    Ok(outputs)
}

/// Unpacks `bundle` (named `name`, manifest `crc`) into `dir`, which should be `library/<name>`.
/// The record is written last; callers treat a present, matching record as "already unpacked".
pub fn unpack_bundle<B: BundleSource>(
    bundle: &B,
    name: &str,
    crc: u32,
    names: &BindingNames,
    dir: &Path,
    options: &UnpackOptions,
) -> Result<UnpackRecord> {
    let objects: BTreeMap<_, ObjectInfo> =
        bundle.objects().into_iter().map(|o| (o.id, o)).collect();
    let assets = bundle.main_assets()?;
    let root = container_root(&assets);

    // Motion metadata (`x.asset` next to `x.anim`) is folded into the clip; movie build data
    // tells which TextAssets are USM parts to join instead of writing them one by one.
    let clip_stems: HashSet<&str> = assets
        .iter()
        .filter_map(|a| a.path.strip_suffix(".anim"))
        .collect();
    let mut metas: BTreeMap<&str, Value> = BTreeMap::new();
    let mut movies: Vec<(&ContainerEntry, Value)> = Vec::new();
    for asset in &assets {
        if !objects
            .get(&asset.id)
            .is_some_and(|o| o.class_id == class_id::MONO_BEHAVIOUR)
        {
            continue;
        }
        let stem = asset.path.strip_suffix(".asset");
        if stem.is_some_and(|stem| clip_stems.contains(stem))
            || asset.path.ends_with("moviebundlebuilddata.asset")
        {
            let tree = bundle.typetree_json(asset.id)?;
            if tree.get("movieBundleDatas").is_some() {
                movies.push((asset, tree));
            } else if let Some(stem) = stem
                && tree.get("FadeInTime").is_some()
            {
                metas.insert(stem, tree);
            }
        }
    }
    let movie_parts: HashSet<String> = movies
        .iter()
        .flat_map(|(_, tree)| {
            tree["movieBundleDatas"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|part| Some(part.get("usmFileName")?.as_str()?.to_lowercase()))
        .collect();
    let is_movie_part = |path: &str| {
        path.rsplit('/')
            .next()
            .is_some_and(|file| movie_parts.contains(file))
    };

    let mut writer = Writer {
        dir,
        taken: HashSet::new(),
        record: UnpackRecord {
            format: unpack::FORMAT.into(),
            version: unpack::VERSION,
            bundle: name.into(),
            crc,
            container_root: root.clone(),
            files: Vec::new(),
            skipped: Vec::new(),
            unresolved_bindings: Vec::new(),
        },
    };
    let mut unresolved = std::collections::BTreeSet::new();
    // One container path can hold several objects (a TMP FontAsset `.asset` also holds its atlas
    // texture and material). The first keeps the plain name; the others become `<stem>.<name>.<ext>`.
    let mut path_uses: BTreeMap<&str, usize> = BTreeMap::new();
    for asset in &assets {
        let Some(object) = objects.get(&asset.id) else {
            writer
                .record
                .skipped
                .push(format!("{}: object {:?} missing", asset.path, asset.id));
            continue;
        };
        if asset
            .path
            .strip_suffix(".asset")
            .is_some_and(|stem| metas.contains_key(stem))
            || is_movie_part(&asset.path)
        {
            continue; // folded into the clip / joined into the movie below
        }
        let uses = path_uses.entry(asset.path.as_str()).or_default();
        *uses += 1;
        let relative_owned;
        let relative = {
            let base = asset
                .path
                .strip_prefix(root.as_str())
                .unwrap_or(&asset.path);
            if *uses == 1 {
                base
            } else {
                let (stem, _) = base.rsplit_once('.').unwrap_or((base, ""));
                let label = object
                    .name
                    .as_deref()
                    .filter(|n| {
                        !n.is_empty() && ripper_format::path::component_problem(n).is_none()
                    })
                    .map_or_else(|| asset.id.path_id.to_string(), str::to_owned);
                relative_owned = format!("{stem}.{label}.{}", asset.id.path_id);
                relative_owned.as_str()
            }
        };
        let converted: Result<Vec<Output>> =
            match object.class_id {
                class_id::FONT => bundle.font_file(asset.id).map_err(ConvertError::from).map(
                    |(bytes, extension)| {
                        let path = if relative.rsplit('/').next().is_some_and(|f| f.contains('.')) {
                            relative.to_owned()
                        } else {
                            format!("{relative}.{extension}")
                        };
                        vec![(path, FileKind::Font, bytes)]
                    },
                ),
                class_id::TEXT_ASSET => bundle
                    .text_asset(asset.id)
                    .map_err(ConvertError::from)
                    .and_then(|bytes| {
                        let path = with_suffix(relative, ".bytes", "");
                        if path.ends_with(".acb") && bytes.starts_with(b"@UTF") {
                            acb_outputs(&path, bytes)
                        } else {
                            Ok(vec![(path, FileKind::Text, bytes)])
                        }
                    }),
                class_id::TEXTURE_2D => bundle
                    .texture_rgba(asset.id)
                    .map_err(ConvertError::from)
                    .and_then(|image| encode_png(&image))
                    .map(|png| {
                        let path = if relative.ends_with(".png") {
                            relative.to_owned()
                        } else {
                            format!("{relative}.png")
                        };
                        vec![(path, FileKind::Png, png)]
                    }),
                class_id::ANIMATION_CLIP => {
                    let stem = asset.path.strip_suffix(".anim").unwrap_or(&asset.path);
                    let source = Source {
                        bundle: name.into(),
                        path_id: asset.id.path_id,
                        container: Some(asset.path.clone()),
                    };
                    bundle
                        .typetree_json(asset.id)
                        .map_err(ConvertError::from)
                        .and_then(|clip| clip_to_motion(&clip, metas.get(stem), names, source))
                        .inspect(|motion| {
                            // Path 0 marks component-level curves (EyeOpening, MouthOpening): nothing to name.
                            unresolved.extend(
                                motion
                                    .curves
                                    .iter()
                                    .filter(|c| {
                                        c.binding.resolved.is_none() && c.binding.path_hash != 0
                                    })
                                    .map(|c| c.binding.path_hash),
                            );
                        })
                        .and_then(|motion| {
                            serde_json::to_vec(&motion)
                                .map_err(|e| malformed("sse-motion", e.to_string()))
                        })
                        .map(|json| {
                            vec![(
                                with_suffix(relative, ".anim", ".sse-motion.json"),
                                FileKind::Motion,
                                json,
                            )]
                        })
                }
                _ => bundle
                    .typetree_json(asset.id)
                    .map_err(ConvertError::from)
                    .and_then(|tree| json_bytes(&tree, "typetree"))
                    .map(|json| {
                        vec![(
                            with_suffix(relative, ".asset", ".json"),
                            FileKind::Typetree,
                            json,
                        )]
                    }),
            };
        match converted {
            Ok(outputs) => {
                for output in outputs {
                    writer.emit(&asset.path, asset.id.path_id, output)?;
                }
            }
            Err(error) => writer
                .record
                .skipped
                .push(format!("{}: {error}", asset.path)),
        }
    }

    for (build_asset, tree) in &movies {
        if let Err(error) = unpack_movie(
            bundle,
            &assets,
            build_asset,
            tree,
            &root,
            options,
            &mut writer,
        ) {
            writer
                .record
                .skipped
                .push(format!("{}: {error}", build_asset.path));
        }
    }

    // Effect prefabs: the prefab root alone is not enough, keep the whole object graph.
    if objects
        .values()
        .any(|o| o.class_id == class_id::GAME_OBJECT)
    {
        let mut graph = serde_json::Map::new();
        for object in objects.values() {
            let tree = bundle
                .typetree_json(object.id)
                .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }));
            graph.insert(
                object.id.path_id.to_string(),
                serde_json::json!({ "classId": object.class_id, "name": object.name, "tree": tree }),
            );
        }
        let bytes = json_bytes(&graph, "object graph")?;
        writer.emit(
            "",
            0,
            ("_objects.json".into(), FileKind::ObjectGraph, bytes),
        )?;
    }

    let mut record = writer.record;
    record.unresolved_bindings = unresolved.into_iter().collect();
    let json = json_bytes(&record, "record")?;
    write_file(dir, unpack::RECORD_FILE, &json)?;
    Ok(record)
}

/// Joins the USM parts of one `MovieBundleBuildData`, demultiplexes them (decision D13) and turns
/// the ADX audio into WAV with ffmpeg when one is configured.
fn unpack_movie<B: BundleSource>(
    bundle: &B,
    assets: &[ContainerEntry],
    build_asset: &ContainerEntry,
    tree: &Value,
    root: &str,
    options: &UnpackOptions,
    writer: &mut Writer<'_>,
) -> Result<()> {
    let part = |file: &str| {
        let file = file.to_lowercase();
        let asset = assets
            .iter()
            .find(|a| a.path.rsplit('/').next() == Some(file.as_str()))?;
        bundle.text_asset(asset.id).ok()
    };
    let usm = crate::movie::assemble_usm(tree, part)?;
    let relative_dir = build_asset
        .path
        .strip_prefix(root)
        .unwrap_or(&build_asset.path)
        .rsplit_once('/')
        .map_or(String::new(), |(dir, _)| format!("{dir}/"));
    let stem = writer
        .record
        .bundle
        .rsplit('/')
        .next()
        .unwrap_or("movie")
        .to_owned();
    for stream in crate::movie::demux_usm(&usm, &stem)? {
        let path = format!("{relative_dir}{}.{}", stream.name, stream.extension);
        let is_adx = stream.extension == "adx";
        writer.emit(
            &build_asset.path,
            build_asset.id.path_id,
            (path.clone(), FileKind::MovieStream, stream.data),
        )?;
        if !is_adx {
            continue;
        }
        let Some(ffmpeg) = &options.ffmpeg else {
            writer.record.skipped.push(format!(
                "{path}: no ffmpeg configured, ADX kept without WAV"
            ));
            continue;
        };
        let wav = with_suffix(&path, ".adx", ".wav");
        match crate::movie::adx_to_wav(
            ffmpeg,
            &path_in(writer.dir, &path),
            &path_in(writer.dir, &wav),
        ) {
            Ok(()) => {
                let bytes = fs::read(path_in(writer.dir, &wav))
                    .map_err(|e| malformed("ffmpeg output", e.to_string()))?;
                writer.emit(
                    &build_asset.path,
                    build_asset.id.path_id,
                    (wav, FileKind::MovieAudio, bytes),
                )?;
            }
            Err(error) => writer.record.skipped.push(format!("{path}: {error}")),
        }
    }
    Ok(())
}

fn path_in(dir: &Path, relative: &str) -> std::path::PathBuf {
    let mut path = dir.to_path_buf();
    path.extend(relative.split('/'));
    path
}

/// Reads an existing record, if any.
pub fn read_record(dir: &Path) -> Option<UnpackRecord> {
    serde_json::from_slice(&fs::read(dir.join(unpack::RECORD_FILE)).ok()?).ok()
}

/// True when `dir` holds a complete unpack of this exact bundle content in the current format and
/// none of its unresolved bindings could now be named.
pub fn is_up_to_date(dir: &Path, crc: u32, names: &BindingNames) -> bool {
    read_record(dir).is_some_and(|r| {
        r.version == unpack::VERSION
            && r.crc == crc
            && !r
                .unresolved_bindings
                .iter()
                .any(|&hash| names.is_known(hash))
            && r.files.iter().all(|f| {
                let mut path = dir.to_path_buf();
                path.extend(f.path.split('/'));
                path.exists()
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ripper_unity::ObjectId;

    fn entry(path: &str) -> ContainerEntry {
        ContainerEntry {
            path: path.into(),
            id: ObjectId {
                file_index: 0,
                path_id: 0,
            },
        }
    }

    #[test]
    fn root_is_the_longest_common_directory() {
        let assets = [
            entry("assets/x/live2d/model/01ichika_normal/a.moc3.bytes"),
            entry("assets/x/live2d/model/01ichika_normal/a.2048/texture_00.png"),
            entry("assets/x/live2d/model/01ichika_normal/motions/m-a.anim"),
        ];
        assert_eq!(
            container_root(&assets),
            "assets/x/live2d/model/01ichika_normal/"
        );
        assert_eq!(container_root(&[entry("a/b/c.png")]), "a/b/");
        assert_eq!(container_root(&[entry("top.png")]), "");
        assert_eq!(container_root(&[]), "");
    }

    /// An in-memory bundle: path id → (class, container, typetree, text bytes).
    struct Fake(Vec<(i64, i32, &'static str, Value, Vec<u8>)>);

    impl BundleSource for Fake {
        fn open(_: &str, _: Vec<u8>, _: &str) -> ripper_unity::Result<Self> {
            unreachable!()
        }
        fn objects(&self) -> Vec<ObjectInfo> {
            self.0
                .iter()
                .map(|(id, class, container, _, _)| ObjectInfo {
                    id: ObjectId {
                        file_index: 0,
                        path_id: *id,
                    },
                    class_id: *class,
                    name: None,
                    container: Some((*container).into()),
                })
                .collect()
        }
        fn typetree_json(&self, id: ObjectId) -> ripper_unity::Result<Value> {
            Ok(self.0.iter().find(|o| o.0 == id.path_id).unwrap().3.clone())
        }
        fn texture_rgba(&self, _: ObjectId) -> ripper_unity::Result<ripper_unity::RgbaImage> {
            Ok(ripper_unity::RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![1, 2, 3, 4],
            })
        }
        fn text_asset(&self, id: ObjectId) -> ripper_unity::Result<Vec<u8>> {
            Ok(self.0.iter().find(|o| o.0 == id.path_id).unwrap().4.clone())
        }
        fn font_file(&self, id: ObjectId) -> ripper_unity::Result<(Vec<u8>, String)> {
            Ok((self.text_asset(id)?, "otf".into()))
        }
        fn content_crc32(&self) -> ripper_unity::Result<u32> {
            Ok(0)
        }
        fn main_assets(&self) -> ripper_unity::Result<Vec<ContainerEntry>> {
            Ok(self
                .0
                .iter()
                .map(|(id, _, container, _, _)| ContainerEntry {
                    path: (*container).into(),
                    id: ObjectId {
                        file_index: 0,
                        path_id: *id,
                    },
                })
                .collect())
        }
    }

    fn clip() -> Value {
        serde_json::json!({
            "m_Name": "w-a01", "m_SampleRate": 60.0,
            "m_MuscleClip": {"m_StartTime": 0.0, "m_StopTime": 1.0, "m_LoopTime": false, "m_Clip": {"data": {
                "m_StreamedClip": {"data": [], "curveCount": 0},
                "m_DenseClip": {"m_CurveCount": 0, "m_SampleRate": 60.0, "m_BeginTime": 0.0, "m_SampleArray": []},
                "m_ConstantClip": {"data": [1.0]}
            }}},
            "m_ClipBindingConstant": {"genericBindings": [{"path": 42, "attribute": 1, "typeID": 114, "customType": 0}]},
            "m_Events": []
        })
    }

    #[test]
    fn unpacks_every_class_and_folds_motion_metadata() {
        let root = "assets/x/live2d/model/m";
        let bundle = Fake(vec![
            (
                1,
                class_id::TEXT_ASSET,
                "assets/x/live2d/model/m/m.moc3.bytes",
                Value::Null,
                b"MOC3".to_vec(),
            ),
            (
                2,
                class_id::TEXTURE_2D,
                "assets/x/live2d/model/m/m.2048/texture_00.png",
                Value::Null,
                vec![],
            ),
            (
                3,
                class_id::ANIMATION_CLIP,
                "assets/x/live2d/model/m/motions/w-a01.anim",
                clip(),
                vec![],
            ),
            (
                4,
                class_id::MONO_BEHAVIOUR,
                "assets/x/live2d/model/m/motions/w-a01.asset",
                serde_json::json!({"FadeInTime": 0.5, "FadeOutTime": 0.25}),
                vec![],
            ),
            (
                5,
                class_id::MONO_BEHAVIOUR,
                "assets/x/live2d/model/m/buildmodeldata.asset",
                serde_json::json!({"m_Name": "BuildModelData"}),
                vec![],
            ),
            (
                6,
                class_id::MONO_BEHAVIOUR,
                "assets/x/live2d/model/m/BuildModelData.asset",
                serde_json::json!({}),
                vec![],
            ),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let record = unpack_bundle(
            &bundle,
            "live2d/model/m",
            9,
            &BindingNames::default(),
            dir.path(),
            &UnpackOptions::default(),
        )
        .unwrap();

        assert_eq!(record.container_root, format!("{root}/"));
        let files: Vec<(&str, FileKind)> = record
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.kind))
            .collect();
        assert_eq!(
            files,
            [
                ("m.moc3", FileKind::Text),
                ("m.2048/texture_00.png", FileKind::Png),
                ("motions/w-a01.sse-motion.json", FileKind::Motion),
                ("buildmodeldata.json", FileKind::Typetree),
            ]
        );
        assert!(
            record.skipped[0].contains("collides"),
            "{:?}",
            record.skipped
        );
        assert_eq!(record.unresolved_bindings, [42]);
        let motion: ripper_format::SseMotion = serde_json::from_slice(
            &fs::read(dir.path().join("motions/w-a01.sse-motion.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(motion.fade.unwrap().fade_out, 0.25);
        assert!(is_up_to_date(dir.path(), 9, &BindingNames::default()));
        assert!(!is_up_to_date(dir.path(), 10, &BindingNames::default()));
        fs::remove_file(dir.path().join("m.moc3")).unwrap();
        assert!(!is_up_to_date(dir.path(), 9, &BindingNames::default()));
    }

    #[test]
    fn suffixes_are_swapped_only_when_present() {
        assert_eq!(with_suffix("a.moc3.bytes", ".bytes", ""), "a.moc3");
        assert_eq!(
            with_suffix("a.physics3.json", ".bytes", ""),
            "a.physics3.json"
        );
        assert_eq!(
            with_suffix("m/w-a.anim", ".anim", ".sse-motion.json"),
            "m/w-a.sse-motion.json"
        );
        assert_eq!(with_suffix("x.prefab", ".asset", ".json"), "x.prefab.json");
    }
}
