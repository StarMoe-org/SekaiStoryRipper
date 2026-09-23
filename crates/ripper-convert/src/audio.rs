//! CRI ACB → per-cue WAV plus metadata (decision D9: WAV + cue metadata + the original ACB).

use std::io::Cursor;

use cridecoder::acb::{UtfTable, Value as UtfValue};
use cridecoder::{HcaDecoder, HcaInfo, extract_acb_to_memory};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{ConvertError, Result};

/// HCA stream parameters, including the loop region CRI stores in the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HcaMeta {
    pub version: u32,
    pub sampling_rate: u32,
    pub channel_count: u32,
    pub block_count: u32,
    pub samples_per_block: usize,
    pub encoder_delay: u32,
    pub encoder_padding: u32,
    pub loop_enabled: bool,
    pub loop_start_block: u32,
    pub loop_end_block: u32,
    pub loop_start_delay: u32,
    pub loop_end_padding: u32,
}

impl From<&HcaInfo> for HcaMeta {
    fn from(info: &HcaInfo) -> Self {
        Self {
            version: info.version,
            sampling_rate: info.sampling_rate,
            channel_count: info.channel_count,
            block_count: info.block_count,
            samples_per_block: info.samples_per_block,
            encoder_delay: info.encoder_delay,
            encoder_padding: info.encoder_padding,
            loop_enabled: info.loop_enabled,
            loop_start_block: info.loop_start_block,
            loop_end_block: info.loop_end_block,
            loop_start_delay: info.loop_start_delay,
            loop_end_padding: info.loop_end_padding,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedCue {
    pub name: String,
    pub cue_id: i32,
    /// `None` when the waveform is not HCA; `wav` then holds the raw waveform bytes.
    pub hca: Option<HcaMeta>,
    /// PCM16 WAV (with a `smpl` loop chunk when the HCA loops).
    pub wav: Vec<u8>,
    pub extension: String,
    /// The HCA stream as stored in the AWB, for archival or reference decoding.
    pub hca_bytes: Option<Vec<u8>>,
}

fn cri(error: impl std::fmt::Display) -> ConvertError {
    ConvertError::Cri(error.to_string())
}

/// Lists the cues of an ACB (with embedded AWB) and decodes each HCA waveform to WAV.
pub fn decode_acb(acb: &[u8]) -> Result<Vec<DecodedCue>> {
    let tracks = extract_acb_to_memory(Cursor::new(acb), None).map_err(cri)?;
    tracks
        .into_iter()
        .map(|track| {
            if track.extension != "hca" {
                return Ok(DecodedCue {
                    name: track.name,
                    cue_id: track.cue_id,
                    hca: None,
                    wav: track.data,
                    extension: track.extension,
                    hca_bytes: None,
                });
            }
            let mut decoder =
                HcaDecoder::from_reader(Cursor::new(track.data.clone())).map_err(cri)?;
            if decoder.info().encryption_enabled {
                return Err(ConvertError::Unsupported("encrypted HCA", track.name));
            }
            let meta = HcaMeta::from(decoder.info());
            let mut wav = Vec::new();
            decoder.decode_to_wav(&mut wav).map_err(cri)?;
            Ok(DecodedCue {
                name: track.name,
                cue_id: track.cue_id,
                hca: Some(meta),
                wav,
                extension: "wav".into(),
                hca_bytes: Some(track.data),
            })
        })
        .collect()
}

/// Dumps every `@UTF` table of an ACB (Cue, CueName, Block, Aisac, ... tables) as JSON, so block and
/// AISAC data survive even though the WAV export cannot express them. Non-table blobs are summarised
/// by size only; waveform data is not copied.
pub fn acb_tables(acb: &[u8]) -> Result<Value> {
    table_json(acb)
}

fn table_json(bytes: &[u8]) -> Result<Value> {
    let table = UtfTable::new(Cursor::new(bytes)).map_err(cri)?;
    let rows = table
        .rows
        .iter()
        .map(map_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({ "name": table.name, "constants": map_json(&table.constants)?, "rows": rows }))
}

fn map_json<S: std::hash::BuildHasher>(
    map: &std::collections::HashMap<String, UtfValue, S>,
) -> Result<Value> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut object = Map::new();
    for key in keys {
        object.insert(key.clone(), value_json(&map[key])?);
    }
    Ok(Value::Object(object))
}

fn value_json(value: &UtfValue) -> Result<Value> {
    Ok(match value {
        UtfValue::U8(v) => json!(v),
        UtfValue::I8(v) => json!(v),
        UtfValue::U16(v) => json!(v),
        UtfValue::I16(v) => json!(v),
        UtfValue::U32(v) => json!(v),
        UtfValue::I32(v) => json!(v),
        UtfValue::U64(v) => json!(v),
        UtfValue::F32(v) => json!(v),
        UtfValue::String(v) => json!(v),
        UtfValue::Data(bytes) if bytes.starts_with(b"@UTF") => table_json(bytes)?,
        UtfValue::Data(bytes) => json!({ "bytes": bytes.len() }),
    })
}
