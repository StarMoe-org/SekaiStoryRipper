use std::io::Write;

use unity_rs_core::{
    bundle::UnityFsBundle, loader::AssetLoadOptions, source::Region, studio::Studio,
    texture::TextureReadLimits,
};

use crate::{BundleSource, ObjectId, ObjectInfo, Result, RgbaImage, UnityError};

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
        let bytes = self
            .object(id)?
            .read_type_tree_json(false, MAX_OBJECT_BYTES)
            .map_err(|error| reader_error(&self.name, error))?;
        serde_json::from_slice(&bytes).map_err(|error| UnityError::Json(id, error))
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
