use std::io::Write;

use unity_rs_core::{
    bundle::UnityFsBundle, loader::AssetLoadOptions, source::Region, studio::Studio,
    texture::TextureReadLimits,
};

use crate::{BundleSource, ObjectId, ObjectInfo, RawTexture, Result, RgbaImage, UnityError};

/// Upper bound for one typetree JSON document or TextAsset (the largest SE pack is ~50 MB).
const MAX_OBJECT_BYTES: usize = 512 << 20;

pub struct UnityRsBundle {
    name: String,
    region: Region,
    studio: Studio,
}

fn reader_error(context: &str, error: impl std::fmt::Display) -> UnityError {
    UnityError::Reader(format!("{context}: {error}"))
}

impl UnityRsBundle {
    fn object(&self, id: ObjectId) -> Result<unity_rs_core::studio::StudioObject<'_>> {
        self.studio
            .object(id.file_index, id.path_id)
            .ok_or(UnityError::MissingObject(id))
    }
}

impl BundleSource for UnityRsBundle {
    fn open(name: &str, bytes: Vec<u8>, unity_version: &str) -> Result<Self> {
        let version = unity_version
            .parse()
            .map_err(|error| reader_error(&format!("unity version {unity_version:?}"), error))?;
        let options = AssetLoadOptions {
            unity_version_override: Some(version),
            ..AssetLoadOptions::default()
        };
        let region = Region::from_bytes(bytes);
        let studio = Studio::open_region_with_options(name, region.clone(), options)
            .map_err(|error| reader_error(name, error))?;
        Ok(Self {
            name: name.to_owned(),
            region,
            studio,
        })
    }

    fn objects(&self) -> Vec<ObjectInfo> {
        self.studio
            .objects()
            .map(|object| ObjectInfo {
                id: ObjectId {
                    file_index: object.file_index(),
                    path_id: object.path_id(),
                },
                class_id: object.class_id(),
                name: object.name().map(str::to_owned),
                container: object.container().map(str::to_owned),
            })
            .collect()
    }

    fn typetree_json(&self, id: ObjectId) -> Result<serde_json::Value> {
        let object = self.object(id)?;
        let bytes = object
            .read_type_tree_json(false, MAX_OBJECT_BYTES)
            .map_err(|error| reader_error(&self.name, error))?;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| UnityError::Json(id, error))?;
        if contains_typeless(&value) {
            // unity-rs renders TypelessData (vertex/index blobs) as `{Offset, Size}` into the
            // object's serialized bytes; inline the bytes so the JSON is self-contained.
            let raw = object
                .read_raw(MAX_OBJECT_BYTES as u64)
                .map_err(|error| reader_error(&self.name, error))?;
            inline_typeless(&mut value, &raw).map_err(|detail| reader_error(&self.name, detail))?;
        }
        Ok(value)
    }

    fn texture_rgba(&self, id: ObjectId) -> Result<RgbaImage> {
        let image = self
            .object(id)?
            .decode_texture_mip(0, TextureReadLimits::default())
            .map_err(|error| reader_error(&self.name, error))?;
        // unity-rs returns Unity's bottom-up row order.
        let stride = image.width as usize * 4;
        let mut pixels = Vec::with_capacity(image.pixels.len());
        for row in image.pixels.chunks_exact(stride).rev() {
            pixels.extend_from_slice(row);
        }
        Ok(RgbaImage {
            width: image.width,
            height: image.height,
            pixels,
        })
    }

    fn texture_raw(&self, id: ObjectId) -> Result<RawTexture> {
        let object = self.object(id)?;
        let collection = self.studio.collection();
        let file = &collection.serialized_files()[object.file_index()].file;
        let texture = unity_rs_core::texture::read_texture2d(
            collection,
            file,
            object.object_index(),
            TextureReadLimits::default(),
        )
        .map_err(|error| reader_error(&self.name, error))?;
        let data = texture
            .data
            .read_to_vec(MAX_OBJECT_BYTES as u64)
            .map_err(|error| reader_error(&self.name, error))?;
        Ok(RawTexture {
            width: texture.width,
            height: texture.height,
            format: texture.format.0,
            data,
        })
    }

    fn text_asset(&self, id: ObjectId) -> Result<Vec<u8>> {
        self.object(id)?
            .read_text_bytes(MAX_OBJECT_BYTES)
            .map_err(|error| reader_error(&self.name, error))
    }

    fn content_crc32(&self) -> Result<u32> {
        content_crc32_region(&self.name, &self.region)
    }

    fn font_file(&self, id: ObjectId) -> Result<(Vec<u8>, String)> {
        let limits = unity_rs_core::simple_assets::SimpleAssetReadLimits {
            maximum_payload_bytes: MAX_OBJECT_BYTES as u64,
            ..Default::default()
        };
        let font = self
            .object(id)?
            .read_font(limits)
            .map_err(|error| reader_error(&self.name, error))?;
        let bytes = font
            .payload
            .read_to_vec(MAX_OBJECT_BYTES as u64)
            .map_err(|error| reader_error(&self.name, error))?;
        Ok((
            bytes,
            font.suggested_extension.trim_start_matches('.').to_owned(),
        ))
    }
}

/// CRC32 over the decompressed entries of a UnityFS bundle in storage order, which is what the
/// CN manifest's `crc` field holds. Only the container is parsed; no SerializedFile is loaded.
pub fn content_crc32(name: &str, unityfs: Vec<u8>) -> Result<u32> {
    content_crc32_region(name, &Region::from_bytes(unityfs))
}

fn content_crc32_region(name: &str, region: &Region) -> Result<u32> {
    struct Crc(crc32fast::Hasher);
    impl Write for Crc {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.update(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bundle = UnityFsBundle::open(region).map_err(|error| reader_error(name, error))?;
    let mut order: Vec<usize> = (0..bundle.entries.len()).collect();
    order.sort_by_key(|&index| bundle.entries[index].offset);
    let mut crc = Crc(crc32fast::Hasher::new());
    for index in order {
        bundle
            .copy_entry(index, &mut crc)
            .map_err(|error| reader_error(name, error))?;
    }
    Ok(crc.0.finalize())
}

fn typeless_span(value: &serde_json::Value) -> Option<(usize, usize)> {
    let object = value.as_object()?;
    if object.len() != 2 {
        return None;
    }
    let offset = object.get("Offset")?.as_u64()?;
    let size = object.get("Size")?.as_u64()?;
    Some((offset as usize, size as usize))
}

fn contains_typeless(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            typeless_span(value).is_some() || map.values().any(contains_typeless)
        }
        serde_json::Value::Array(items) => items.iter().any(contains_typeless),
        _ => false,
    }
}

/// Replaces every `{Offset, Size}` TypelessData node with the byte array it points at.
fn inline_typeless(value: &mut serde_json::Value, raw: &[u8]) -> std::result::Result<(), String> {
    if let Some((offset, size)) = typeless_span(value) {
        let bytes = raw.get(offset..offset + size).ok_or_else(|| {
            format!(
                "TypelessData {offset}+{size} outside the {}-byte object",
                raw.len()
            )
        })?;
        *value =
            serde_json::Value::Array(bytes.iter().map(|&b| serde_json::Value::from(b)).collect());
        return Ok(());
    }
    match value {
        serde_json::Value::Object(map) => {
            map.values_mut().try_for_each(|v| inline_typeless(v, raw))
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().try_for_each(|v| inline_typeless(v, raw))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typeless_nodes_are_replaced_by_their_bytes() {
        let mut value = serde_json::json!({"m_VertexData": {"m_DataSize": {"Offset": 2, "Size": 3}}, "keep": {"Offset": 1}});
        assert!(contains_typeless(&value));
        inline_typeless(&mut value, &[9, 9, 1, 2, 3, 9]).unwrap();
        assert_eq!(
            value["m_VertexData"]["m_DataSize"],
            serde_json::json!([1, 2, 3])
        );
        assert_eq!(value["keep"], serde_json::json!({"Offset": 1}));
        let mut bad = serde_json::json!({"Offset": 5, "Size": 5});
        assert!(inline_typeless(&mut bad, &[0; 4]).is_err());
    }
}
