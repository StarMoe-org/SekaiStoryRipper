//! `ripper-acb` v1: how an unpacked CRI ACB maps cue names to decoded waveforms.
//!
//! For `library/<bundle>/<x>.acb` the unpacker writes, next to the raw ACB (kept for block/AISAC
//! semantics, decision D9):
//! - `<x>.cues.json`: this document;
//! - `<x>.audio/<waveform>.wav`: every physical waveform once (decision: export per waveform, since
//!   block BGMs have many cues/tracks sharing and reusing waveforms);
//! - `<x>.tables.json`: every `@UTF` table of the ACB (Cue, Block, Track, Aisac, ...).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ripper-acb";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcbIndex {
    pub format: String,
    pub version: u32,
    /// ACB file name the index belongs to.
    pub acb: String,
    /// Cues in ACB track order; a cue lists its waveforms in track order (one for voice/SE).
    pub cues: Vec<Cue>,
    /// Keyed by waveform id (`e<id>` embedded AWB, `s<awb>_<id>` streaming AWB).
    pub waveforms: BTreeMap<String, Waveform>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cue {
    pub name: String,
    pub cue_id: i32,
    pub waveforms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    /// Relative to the ACB's directory, e.g. `x.audio/e12.wav`.
    pub file: String,
    /// Source encoding (`hca`, `adx`, ...). Non-HCA waveforms are written raw with that extension.
    pub encoding: String,
    pub hca: Option<HcaInfo>,
}

/// HCA stream parameters, including the loop region CRI stores in the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HcaInfo {
    pub version: u32,
    pub sampling_rate: u32,
    pub channel_count: u32,
    pub block_count: u32,
    pub samples_per_block: u32,
    pub encoder_delay: u32,
    pub encoder_padding: u32,
    pub loop_enabled: bool,
    pub loop_start_block: u32,
    pub loop_end_block: u32,
    pub loop_start_delay: u32,
    pub loop_end_padding: u32,
}

impl AcbIndex {
    /// The waveform files of a cue, looked up by name as the game does.
    pub fn cue_files(&self, cue_name: &str) -> Option<Vec<&str>> {
        let cue = self.cues.iter().find(|c| c.name == cue_name)?;
        Some(
            cue.waveforms
                .iter()
                .filter_map(|w| Some(self.waveforms.get(w)?.file.as_str()))
                .collect(),
        )
    }
}
