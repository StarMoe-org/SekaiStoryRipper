//! Reading Unity AssetBundles.
//!
//! Everything above this crate talks to [`BundleSource`], so the reader can be swapped (for
//! example for a minimal in-house UnityFS reader) without touching the converters.
//! The only implementation today is [`UnityRsBundle`], backed by `unity-rs-core`.

mod unity_rs;

pub use unity_rs::UnityRsBundle;

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
}
