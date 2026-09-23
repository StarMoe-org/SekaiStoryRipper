//! `ripper-unpack` v1: the record written next to every unpacked bundle in the library.
//!
//! Library layout: `library/<bundleName>/<container path relative to the bundle's root>`, so
//! relative references inside the assets (e.g. `model3.json` → `…/texture_00.png`) keep working.
//! The record lists the files, and its presence with a matching `crc` and `version` means the
//! bundle is fully unpacked and can be skipped.

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ripper-unpack";
pub const VERSION: u32 = 1;
/// File name of the record inside `library/<bundleName>/`.
pub const RECORD_FILE: &str = "_ripper.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpackRecord {
    pub format: String,
    pub version: u32,
    pub bundle: String,
    /// Manifest crc of the unpacked bundle.
    pub crc: u32,
    /// Container prefix that was stripped to get the relative file paths.
    pub container_root: String,
    pub files: Vec<UnpackedFile>,
    /// Objects that were not converted, with the reason.
    pub skipped: Vec<String>,
    /// Binding path hashes of this bundle's clips that no known moc3 id matched, sorted. When a
    /// later model makes one of them resolvable, the bundle is unpacked again.
    #[serde(default)]
    pub unresolved_bindings: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpackedFile {
    /// Relative to `library/<bundleName>/`, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    /// Original container path of the object.
    pub container: String,
    pub path_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    /// `TextAsset` bytes as stored (`.bytes` suffix dropped), e.g. moc3, model3/physics3 JSON.
    Text,
    /// `Texture2D` decoded to RGBA PNG.
    Png,
    /// ASTC texture's original blocks (mip 0) as a `.astc` file, Unity row order (opt-in).
    Astc,
    /// `AnimationClip` as `sse-motion` JSON.
    Motion,
    /// Any other object's embedded typetree as JSON (ScenarioSceneData, BuildModelData, prefabs, ...).
    Typetree,
    /// `<x>.cues.json` (`ripper-acb` index) of an ACB TextAsset.
    AcbIndex,
    /// `<x>.tables.json`: every UTF table of an ACB.
    AcbTables,
    /// A waveform of an ACB: WAV for HCA, raw otherwise.
    Audio,
    /// A demultiplexed stream of a story movie (`.m2v` video, `.adx` audio).
    MovieStream,
    /// A movie's ADX audio converted to WAV by ffmpeg.
    MovieAudio,
    /// A `Font`'s embedded font file (`.otf` / `.ttf`).
    Font,
    /// `_objects.json`: typetrees of every object in a bundle with GameObjects (effect prefabs).
    ObjectGraph,
}
