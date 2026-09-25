//! CRI ACB → one WAV per physical waveform + `ripper-acb` index + all UTF tables (ADR-0007).

use std::collections::BTreeMap;
use std::io::Cursor;

use cridecoder::HcaDecoder;
use cridecoder::acb::{AfsArchive, TrackList, UtfTable, Value as UtfValue};
use ripper_format::audio::{self, AcbIndex, Cue, HcaInfo, Waveform};
use serde_json::{Map, Value, json};

use crate::{ConvertError, Result};

fn cri(error: impl std::fmt::Display) -> ConvertError {
    ConvertError::Cri(error.to_string())
}

/// Everything derived from one ACB.
pub struct AcbExport {
    pub index: AcbIndex,
    /// `(path relative to the ACB's directory, bytes)` for each waveform.
    pub files: Vec<(String, Vec<u8>)>,
    /// All `@UTF` tables as JSON.
    pub tables: Value,
}

fn hca_info(info: &cridecoder::HcaInfo) -> HcaInfo {
    HcaInfo {
        version: info.version,
        sampling_rate: info.sampling_rate,
        channel_count: info.channel_count,
        block_count: info.block_count,
        samples_per_block: info.samples_per_block as u32,
        encoder_delay: info.encoder_delay,
        encoder_padding: info.encoder_padding,
        loop_enabled: info.loop_enabled,
        loop_start_block: info.loop_start_block,
        loop_end_block: info.loop_end_block,
        loop_start_delay: info.loop_start_delay,
        loop_end_padding: info.loop_end_padding,
    }
}

/// Decodes an HCA stream to PCM16 WAV (with a `smpl` loop chunk when it loops).
pub fn hca_to_wav(hca: Vec<u8>) -> Result<(Vec<u8>, HcaInfo)> {
    let mut decoder = HcaDecoder::from_reader(Cursor::new(hca)).map_err(cri)?;
    if decoder.info().encryption_enabled {
        return Err(ConvertError::Unsupported("encrypted HCA", String::new()));
    }
    let info = hca_info(decoder.info());
    let mut wav = Vec::new();
    decoder.decode_to_wav(&mut wav).map_err(cri)?;
    Ok((wav, info))
}

/// `acb_file` is the ACB's file name (e.g. `se_pack00001_b.acb`); waveforms go to `<stem>.audio/`.
pub fn export_acb(acb_file: &str, acb: &[u8]) -> Result<AcbExport> {
    let stem = acb_file.strip_suffix(".acb").unwrap_or(acb_file);
    let mut utf = UtfTable::new(Cursor::new(acb)).map_err(cri)?;
    let tracks = TrackList::new(&utf).map_err(cri)?.tracks;
    // Track names from TrackList are per track (`bgm90001-12`); the game plays cues by the
    // CueNameTable name, so group tracks by cue id and name each cue from that table.
    let cue_names: BTreeMap<i64, String> =
        match utf.rows.first().and_then(|row| row.get("CueNameTable")) {
            Some(UtfValue::Data(bytes)) if bytes.starts_with(b"@UTF") => {
                UtfTable::new(Cursor::new(bytes))
                    .map_err(cri)?
                    .rows
                    .iter()
                    .filter_map(|row| {
                        Some((
                            row.get("CueIndex")?.as_int()?,
                            row.get("CueName")?.as_string()?.to_owned(),
                        ))
                    })
                    .collect()
            }
            _ => BTreeMap::new(),
        };
    let mut embedded = match utf.rows.first_mut().and_then(|row| row.remove("AwbFile")) {
        Some(UtfValue::Data(bytes)) if !bytes.is_empty() => {
            Some(AfsArchive::new(Cursor::new(bytes)).map_err(cri)?)
        }
        _ => None,
    };

    let mut index = AcbIndex {
        format: audio::FORMAT.into(),
        version: audio::VERSION,
        acb: acb_file.into(),
        cues: Vec::new(),
        waveforms: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let mut files = Vec::new();
    let mut cue_position: BTreeMap<i32, usize> = BTreeMap::new();
    for track in &tracks {
        let key = if track.is_stream {
            format!("s{}_{}", track.stream_awb_id, track.wav_id)
        } else {
            format!("e{}", track.wav_id)
        };
        let position = *cue_position.entry(track.cue_id).or_insert_with(|| {
            let name = cue_names
                .get(&i64::from(track.cue_id))
                .cloned()
                .unwrap_or_else(|| track.name.clone());
            index.cues.push(Cue {
                name,
                cue_id: track.cue_id,
                waveforms: Vec::new(),
            });
            index.cues.len() - 1
        });
        index.cues[position].waveforms.push(key.clone());
        if index.waveforms.contains_key(&key) {
            continue;
        }
        if track.is_stream {
            // Streaming AWBs are separate files; the story ACBs seen so far embed everything.
            index.warnings.push(format!(
                "{}: waveform {key} is in a streaming AWB that is not available",
                track.name
            ));
            continue;
        }
        let Some(awb) = embedded.as_mut() else {
            index
                .warnings
                .push(format!("{}: ACB has no embedded AWB", track.name));
            continue;
        };
        let data = awb.file_data_for_cue_id(track.wav_id).map_err(cri)?;
        let encoding = cridecoder::acb::wave_type_extension(track.enc_type)
            .trim_start_matches('.')
            .to_owned();
        let (file, bytes, hca) = if encoding == "hca" {
            let (wav, info) = hca_to_wav(data)?;
            (format!("{stem}.audio/{key}.wav"), wav, Some(info))
        } else {
            let extension = if encoding.is_empty() {
                track.enc_type.to_string()
            } else {
                encoding.clone()
            };
            (format!("{stem}.audio/{key}.{extension}"), data, None)
        };
        files.push((file.clone(), bytes));
        index.waveforms.insert(
            key,
            Waveform {
                file,
                encoding,
                hca,
            },
        );
    }
    Ok(AcbExport {
        index,
        files,
        tables: acb_tables(acb)?,
    })
}

/// Dumps every `@UTF` table of an ACB as JSON. Non-table blobs (the AWB) are summarised by size.
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
