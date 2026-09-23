//! Reading Unity AssetBundles.
//!
//! Everything above this crate talks to [`BundleSource`], so the reader can be swapped (for
//! example for a minimal in-house UnityFS reader) without touching the converters.
//! The only implementation today is [`UnityRsBundle`], backed by `unity-rs-core`.

mod unity_rs;

pub use unity_rs::{UnityRsBundle, content_crc32};

/// Unity version of the CN 6.4.0 client. Bundle headers carry a stripped placeholder instead.
pub const DEFAULT_UNITY_VERSION: &str = "2022.3.62f3";

/// Unity class ids used by the story assets.
pub mod class_id {
    pub const TEXTURE_2D: i32 = 28;
    pub const TEXT_ASSET: i32 = 49;
    pub const ANIMATION_CLIP: i32 = 74;
    pub const MONO_BEHAVIOUR: i32 = 114;
    pub const MONO_SCRIPT: i32 = 115;
    pub const ASSET_BUNDLE: i32 = 142;
    pub const SPRITE: i32 = 213;
}

#[derive(Debug, thiserror::Error)]
pub enum UnityError {
    #[error("{0}")]
    Reader(String),
    #[error("object {0:?} not found")]
    MissingObject(ObjectId),
    #[error("typetree of {0:?} is not valid JSON: {1}")]
    Json(ObjectId, serde_json::Error),
}

pub type Result<T> = std::result::Result<T, UnityError>;

/// Identifies one object: serialized-file index inside the bundle plus its path id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    pub file_index: usize,
    pub path_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    pub id: ObjectId,
    pub class_id: i32,
    pub name: Option<String>,
    /// `AssetBundle.m_Container` path, lower-cased by Unity at build time.
    pub container: Option<String>,
}

/// One `AssetBundle.m_Container` entry: the path a `Contains()`/`LoadAsset(path)` hits and its main asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    /// Lower-cased by Unity at build time, e.g. `assets/.../motions/w-cute-nod05.anim`.
    pub path: String,
    pub id: ObjectId,
}

/// Tightly packed RGBA8 pixels, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub trait BundleSource: Sized {
    /// Opens a deobfuscated UnityFS bundle. `name` is only used in diagnostics.
    fn open(name: &str, bytes: Vec<u8>, unity_version: &str) -> Result<Self>;

    fn objects(&self) -> Vec<ObjectInfo>;

    /// The object's embedded typetree rendered as JSON.
    fn typetree_json(&self, id: ObjectId) -> Result<serde_json::Value>;

    /// Decodes mip 0 of a `Texture2D`.
    fn texture_rgba(&self, id: ObjectId) -> Result<RgbaImage>;

    /// Raw `m_Script` bytes of a `TextAsset` (moc3, json, acb, ...).
    fn text_asset(&self, id: ObjectId) -> Result<Vec<u8>>;

    /// CRC32 over the decompressed bundle entries in storage order; equals the manifest `crc`.
    fn content_crc32(&self) -> Result<u32>;

    /// The `m_Container` of the bundle's `AssetBundle` object, in stored order. Unlike
    /// [`ObjectInfo::container`] (which also tags objects in an entry's preload range), each path
    /// maps to exactly the asset Unity returns for it.
    fn main_assets(&self) -> Result<Vec<ContainerEntry>> {
        let objects = self.objects();
        let Some(bundle) = objects
            .iter()
            .find(|o| o.class_id == class_id::ASSET_BUNDLE)
        else {
            return Ok(Vec::new());
        };
        let tree = self.typetree_json(bundle.id)?;
        let entries = tree
            .get("m_Container")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(entries.len());
        for entry in entries {
            let path = entry.get("key").and_then(serde_json::Value::as_str);
            let asset = entry.get("value").and_then(|v| v.get("asset"));
            let file_id = asset
                .and_then(|a| a.get("m_FileID"))
                .and_then(serde_json::Value::as_i64);
            let path_id = asset
                .and_then(|a| a.get("m_PathID"))
                .and_then(serde_json::Value::as_i64);
            match (path, file_id, path_id) {
                // m_FileID 0 = the AssetBundle object's own serialized file.
                (Some(path), Some(0), Some(path_id)) => out.push(ContainerEntry {
                    path: path.to_owned(),
                    id: ObjectId {
                        file_index: bundle.id.file_index,
                        path_id,
                    },
                }),
                (Some(path), Some(file_id), Some(_)) => {
                    return Err(UnityError::Reader(format!(
                        "{path}: asset in external file {file_id} is not supported"
                    )));
                }
                _ => {
                    return Err(UnityError::Reader(format!(
                        "malformed m_Container entry {entry}"
                    )));
                }
            }
        }
        Ok(out)
    }
}
