//! `ripper-objects` v1: `_objects.json`, the whole object graph of a bundle with GameObjects.
//!
//! Effect prefabs are only usable as a graph (transforms, components, materials, sprites,
//! animator controllers reference each other by path id), so the unpacker writes every object of
//! such a bundle into one document next to the per-asset files. `tree` is the object's embedded
//! typetree as Unity serialized it: its schema is the game's, fixed by the lock's `unityVersion`,
//! not by this format.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ripper-objects";
pub const VERSION: u32 = 1;
/// File name inside `library/<bundleName>/`.
pub const FILE: &str = "_objects.json";

/// `T` is the typetree value (`serde_json::Value` for a JSON reader).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectGraph<T> {
    pub format: String,
    pub version: u32,
    /// Keyed by path id (decimal).
    pub objects: BTreeMap<String, Object<T>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Object<T> {
    pub class_id: i32,
    pub name: Option<String>,
    pub tree: T,
}
