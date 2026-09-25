//! CRI USM demultiplexing (ADR-0007: demux only, no transcoding).

use std::io::Cursor;

use cridecoder::extract_usm_to_memory;

use crate::{ConvertError, Result};

#[derive(Debug, Clone)]
pub struct UsmStream {
    /// File name suggested by the USM (`<name>.<extension>`).
    pub name: String,
    /// Container/codec hint from the USM header, e.g. `m2v`, `ivf`, `h264`, `hca`, `adx`.
    pub extension: String,
    pub data: Vec<u8>,
}

/// Story movies are split into several `<name>-NNN.usm` TextAssets. `MovieBundleBuildData`
/// (`movieBundleDatas[] { index, usmFileName }`, `totalbyte`) gives their order and joined size.
pub fn assemble_usm(
    build_data: &serde_json::Value,
    part: impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<Vec<u8>> {
    let malformed = |detail: String| crate::malformed("MovieBundleBuildData", detail);
    let mut parts: Vec<(i64, &str)> = build_data
        .get("movieBundleDatas")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| malformed("missing movieBundleDatas".into()))?
        .iter()
        .map(|entry| {
            let index = entry.get("index").and_then(serde_json::Value::as_i64);
            let file = entry.get("usmFileName").and_then(serde_json::Value::as_str);
            index
                .zip(file)
                .ok_or_else(|| malformed(format!("bad entry {entry}")))
        })
        .collect::<Result<_>>()?;
    parts.sort_by_key(|&(index, _)| index);
    let mut joined = Vec::new();
    for (_, file) in parts {
        joined.extend(part(file).ok_or_else(|| malformed(format!("part {file} not in bundle")))?);
    }
    let total = build_data
        .get("totalbyte")
        .and_then(serde_json::Value::as_u64);
    if total != Some(joined.len() as u64) {
        return Err(malformed(format!(
            "joined {} bytes, totalbyte says {total:?}",
            joined.len()
        )));
    }
    Ok(joined)
}

pub fn demux_usm(usm: &[u8], fallback_name: &str) -> Result<Vec<UsmStream>> {
    let streams = extract_usm_to_memory(Cursor::new(usm), fallback_name.as_bytes(), None, true)
        .map_err(|error| ConvertError::Cri(error.to_string()))?;
    Ok(streams
        .into_iter()
        .map(|s| UsmStream {
            name: s.name,
            extension: s.extension,
            data: s.data,
        })
        .collect())
}

/// Converts a demultiplexed CRI ADX stream to PCM16 WAV with an external ffmpeg (cridecoder only
/// demultiplexes it; ADR-0007).
pub fn adx_to_wav(
    ffmpeg: &std::path::Path,
    adx: &std::path::Path,
    wav: &std::path::Path,
) -> Result<()> {
    let output = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(adx)
        .args(["-c:a", "pcm_s16le"])
        .arg(wav)
        .output()
        .map_err(|error| ConvertError::Cri(format!("running {}: {error}", ffmpeg.display())))?;
    if !output.status.success() {
        return Err(ConvertError::Cri(format!(
            "ffmpeg failed on {}: {}",
            adx.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// The configured ffmpeg if it runs (`ffmpeg -version`), else `None`.
pub fn find_ffmpeg(configured: &std::path::Path) -> Option<std::path::PathBuf> {
    let ok = std::process::Command::new(configured)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    ok.then(|| configured.to_path_buf())
}
