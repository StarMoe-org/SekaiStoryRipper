//! Parameter and part ids of a `.moc3`, needed to name AnimationClip bindings (ADR-0008).
//!
//! Only the id tables are read; the model itself is never evaluated here. Layout (little-endian
//! unless the endian flag at byte 5 is set): magic `MOC3`, version byte at 4, then a section
//! offset table at 64. Entry 0 points to the count table (parts at +0, parameters at +20), entry 3
//! (byte 76) to the part ids and entry 50 (byte 264) to the parameter ids; every id is a
//! NUL-padded 64-byte string. Same layout as `unity-rs-core`'s `cubism_moc` reader (MIT).

use crate::{Result, malformed};

const MAGIC: &[u8; 4] = b"MOC3";
const COUNT_TABLE_OFFSET: usize = 64;
const PART_IDS_OFFSET: usize = 76;
const PARAMETER_IDS_OFFSET: usize = 264;
const PART_COUNT_IN_TABLE: usize = 0;
const PARAMETER_COUNT_IN_TABLE: usize = 20;
const ID_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MocIds {
    /// Version byte (2 = moc3 3.3 on CN 6.4.0).
    pub version: u8,
    /// In file order.
    pub parameters: Vec<String>,
    /// In file order.
    pub parts: Vec<String>,
}

pub fn read_moc3_ids(data: &[u8]) -> Result<MocIds> {
    if data.len() < PARAMETER_IDS_OFFSET + 4 || &data[..4] != MAGIC {
        return Err(malformed("moc3", "missing MOC3 header"));
    }
    let big_endian = data[5] != 0;
    let u32_at = |offset: usize| -> Result<usize> {
        let bytes: [u8; 4] = data
            .get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| malformed("moc3", format!("offset {offset} out of range")))?;
        let value = if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        };
        Ok(value as usize)
    };
    let ids_at = |table: usize, count: usize| -> Result<Vec<String>> {
        (0..count)
            .map(|index| {
                let start = table + index * ID_BYTES;
                let raw = data
                    .get(start..start + ID_BYTES)
                    .ok_or_else(|| malformed("moc3", format!("id {index} out of range")))?;
                let length = raw.iter().position(|&b| b == 0).unwrap_or(ID_BYTES);
                Ok(String::from_utf8_lossy(&raw[..length]).into_owned())
            })
            .collect()
    };
    let counts = u32_at(COUNT_TABLE_OFFSET)?;
    let part_count = u32_at(counts + PART_COUNT_IN_TABLE)?;
    let parameter_count = u32_at(counts + PARAMETER_COUNT_IN_TABLE)?;
    Ok(MocIds {
        version: data[4],
        parts: ids_at(u32_at(PART_IDS_OFFSET)?, part_count)?,
        parameters: ids_at(u32_at(PARAMETER_IDS_OFFSET)?, parameter_count)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ids_from_a_synthetic_moc3() {
        let mut data = vec![0u8; 1024];
        data[..4].copy_from_slice(MAGIC);
        data[4] = 2;
        let (counts, parts, params) = (300usize, 400usize, 600usize);
        data[COUNT_TABLE_OFFSET..COUNT_TABLE_OFFSET + 4]
            .copy_from_slice(&(counts as u32).to_le_bytes());
        data[PART_IDS_OFFSET..PART_IDS_OFFSET + 4].copy_from_slice(&(parts as u32).to_le_bytes());
        data[PARAMETER_IDS_OFFSET..PARAMETER_IDS_OFFSET + 4]
            .copy_from_slice(&(params as u32).to_le_bytes());
        data[counts..counts + 4].copy_from_slice(&1u32.to_le_bytes());
        data[counts + 20..counts + 24].copy_from_slice(&2u32.to_le_bytes());
        data[parts..parts + 8].copy_from_slice(b"PartHair");
        data[params..params + 11].copy_from_slice(b"ParamAngleX");
        data[params + 64..params + 75].copy_from_slice(b"ParamAngleY");

        let ids = read_moc3_ids(&data).unwrap();
        assert_eq!(ids.version, 2);
        assert_eq!(ids.parts, ["PartHair"]);
        assert_eq!(ids.parameters, ["ParamAngleX", "ParamAngleY"]);
    }

    #[test]
    fn rejects_non_moc3_data() {
        assert!(read_moc3_ids(&[0u8; 400]).is_err());
    }
}
