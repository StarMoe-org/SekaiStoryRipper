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

/// Unpacks `bundle` (named `name`, manifest `crc`) into `dir`, which should be `library/<name>`.
/// The record is written last; callers treat a present, matching record as "already unpacked".
pub fn unpack_bundle<B: BundleSource>(
    bundle: &B,
    name: &str,
    crc: u32,
    names: &BindingNames,
    dir: &Path,
) -> Result<UnpackRecord> {
    let objects: BTreeMap<_, ObjectInfo> =
        bundle.objects().into_iter().map(|o| (o.id, o)).collect();
    let assets = bundle.main_assets()?;
    let root = container_root(&assets);

    // Motion metadata (`x.asset` next to `x.anim`) is folded into the clip.
    let clip_stems: HashSet<&str> = assets
        .iter()
        .filter_map(|a| a.path.strip_suffix(".anim"))
        .collect();
    let mut metas: BTreeMap<&str, Value> = BTreeMap::new();
    for asset in &assets {
        if let Some(stem) = asset.path.strip_suffix(".asset")
            && clip_stems.contains(stem)
            && objects
                .get(&asset.id)
                .is_some_and(|o| o.class_id == class_id::MONO_BEHAVIOUR)
        {
            let tree = bundle.typetree_json(asset.id)?;
            if tree.get("FadeInTime").is_some() {
                metas.insert(stem, tree);
            }
        }
    }

    let mut record = UnpackRecord {
        format: unpack::FORMAT.into(),
        version: unpack::VERSION,
        bundle: name.into(),
        crc,
        container_root: root.clone(),
        files: Vec::new(),
        skipped: Vec::new(),
        unresolved_bindings: Vec::new(),
    };
    let mut unresolved = std::collections::BTreeSet::new();
    let mut taken: HashSet<String> = HashSet::new();
    for asset in &assets {
        let Some(object) = objects.get(&asset.id) else {
            record
                .skipped
                .push(format!("{}: object {:?} missing", asset.path, asset.id));
            continue;
        };
        if asset
            .path
            .strip_suffix(".asset")
            .is_some_and(|stem| metas.contains_key(stem))
        {
            continue; // folded into the clip
        }
        let relative = asset
            .path
            .strip_prefix(root.as_str())
            .unwrap_or(&asset.path);
        let converted: Result<(String, FileKind, Vec<u8>)> = match object.class_id {
            class_id::TEXT_ASSET => bundle
                .text_asset(asset.id)
                .map(|bytes| (with_suffix(relative, ".bytes", ""), FileKind::Text, bytes))
                .map_err(ConvertError::from),
            class_id::TEXTURE_2D => bundle
                .texture_rgba(asset.id)
                .map_err(ConvertError::from)
                .and_then(|image| encode_png(&image))
                .map(|png| {
                    (
                        if relative.ends_with(".png") {
                            relative.to_owned()
                        } else {
                            format!("{relative}.png")
                        },
                        FileKind::Png,
                        png,
                    )
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
                        (
                            with_suffix(relative, ".anim", ".sse-motion.json"),
                            FileKind::Motion,
                            json,
                        )
                    })
            }
            _ => bundle
                .typetree_json(asset.id)
                .map_err(ConvertError::from)
                .and_then(|tree| {
                    serde_json::to_vec_pretty(&tree)
                        .map_err(|e| malformed("typetree", e.to_string()))
                })
                .map(|json| {
                    (
                        with_suffix(relative, ".asset", ".json"),
                        FileKind::Typetree,
                        json,
                    )
                }),
        };
        let (path, kind, bytes) = match converted {
            Ok(converted) => converted,
            Err(error) => {
                record.skipped.push(format!("{}: {error}", asset.path));
                continue;
            }
        };
        if let Err(problem) = ripper_format::path::check_relative(&path) {
            record.skipped.push(format!("{}: {problem}", asset.path));
            continue;
        }
        // macOS and Windows file systems are case-insensitive by default.
        if !taken.insert(path.to_lowercase()) {
            record.skipped.push(format!(
                "{}: {path} collides (case-insensitively) with another file",
                asset.path
            ));
            continue;
        }
        write_file(dir, &path, &bytes)?;
        record.files.push(UnpackedFile {
            path,
            kind,
            container: asset.path.clone(),
            path_id: asset.id.path_id,
        });
    }
    record.unresolved_bindings = unresolved.into_iter().collect();
    let json =
        serde_json::to_vec_pretty(&record).map_err(|e| malformed("record", e.to_string()))?;
    write_file(dir, unpack::RECORD_FILE, &json)?;
    Ok(record)
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
