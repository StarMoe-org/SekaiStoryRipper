//! `ripper-lock` v1: `ripper.lock.json` at the output root, written last by every run.
//!
//! It is the contract between a library and its readers: `formats` names the version of every
//! format the library's documents are written in ([`crate::formats`] at write time). A reader
//! compares it with the versions it was built against before reading anything else, and refuses
//! a library that differs instead of guessing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ripper-lock";
pub const VERSION: u32 = 1;
/// File name at the output root, next to `library/` and `episodes/`.
pub const FILE: &str = "ripper.lock.json";

/// `M` is the masterdata provenance (`serde_json::Value` when writing; a reader that does not
/// need it can use `serde::de::IgnoredAny`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lock<M> {
    pub format: String,
    pub version: u32,
    pub tool_version: String,
    /// Format name → version of the documents in this library.
    pub formats: BTreeMap<String, u32>,
    pub region: String,
    pub app_version: String,
    pub asset_version: String,
    pub unity_version: String,
    pub masterdata: M,
    pub episodes: Vec<String>,
}
