//! `AssetBundleInfoNew.json`: AES-128-CBC (ABCrypt) + PKCS#7, then msgpack.
//!
//! The ABCrypt key and IV are never shipped with this tool (decision D4); callers supply them.

use std::collections::BTreeMap;

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use serde::Deserialize;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("ABCrypt key and IV must be 16 bytes each (got {key} and {iv})")]
    KeyLength { key: usize, iv: usize },
    #[error("manifest decryption failed (wrong ABCrypt key/IV?)")]
    Decrypt,
    #[error("manifest msgpack is malformed: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
}

/// ABCrypt key material, provided by the user.
#[derive(Clone)]
pub struct ManifestKey {
    key: [u8; 16],
    iv: [u8; 16],
}

impl ManifestKey {
    pub fn new(key: &[u8], iv: &[u8]) -> Result<Self, ManifestError> {
        let key_len = key.len();
        let iv_len = iv.len();
        match (<[u8; 16]>::try_from(key), <[u8; 16]>::try_from(iv)) {
            (Ok(key), Ok(iv)) => Ok(Self { key, iv }),
            _ => Err(ManifestError::KeyLength {
                key: key_len,
                iv: iv_len,
            }),
        }
    }
}

impl std::fmt::Debug for ManifestKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManifestKey(<redacted>)")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub bundles: BTreeMap<String, BundleEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub bundle_name: String,
    #[serde(default)]
    pub category: Option<String>,
    /// Size of the UnityFS payload; a download is `file_size + 4` bytes.
    pub file_size: u64,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// CDN directory holding this bundle (`ios1`, `ios10`, ...).
    pub download_path: String,
    /// CRC32 of the concatenated, decompressed bundle entries (verified in M0, see docs/spike/M0-report.md).
    pub crc: u32,
}

impl Manifest {
    pub fn decrypt(ciphertext: &[u8], key: &ManifestKey) -> Result<Self, ManifestError> {
        let plain = Aes128CbcDec::new(&key.key.into(), &key.iv.into())
            .decrypt_padded_vec::<Pkcs7>(ciphertext)
            .map_err(|_| ManifestError::Decrypt)?;
        Self::from_msgpack(&plain)
    }

    pub fn from_msgpack(plain: &[u8]) -> Result<Self, ManifestError> {
        Ok(rmp_serde::from_slice(plain)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockModeEncrypt;

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    #[test]
    fn decrypts_an_encrypted_msgpack_manifest() {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry<'a> {
            bundle_name: &'a str,
            file_size: u64,
            download_path: &'a str,
            crc: u32,
            dependencies: Vec<&'a str>,
            hash: Option<&'a str>,
        }
        #[derive(serde::Serialize)]
        struct Doc<'a> {
            version: Option<u8>,
            bundles: BTreeMap<&'a str, Entry<'a>>,
        }
        let name = "music/music_score/0074_01";
        let doc = Doc {
            version: None,
            bundles: BTreeMap::from([(
                name,
                Entry {
                    bundle_name: name,
                    file_size: 9611,
                    download_path: "ios1",
                    crc: 1_715_166_487,
                    dependencies: vec![],
                    hash: None,
                },
            )]),
        };
        let plain = rmp_serde::to_vec_named(&doc).unwrap();
        let key = ManifestKey::new(b"0123456789abcdef", b"fedcba9876543210").unwrap();
        let ciphertext =
            Aes128CbcEnc::new(&key.key.into(), &key.iv.into()).encrypt_padded_vec::<Pkcs7>(&plain);

        let manifest = Manifest::decrypt(&ciphertext, &key).unwrap();
        let entry = &manifest.bundles[name];
        assert_eq!(
            (entry.file_size, entry.crc, entry.download_path.as_str()),
            (9611, 1_715_166_487, "ios1")
        );

        let wrong = ManifestKey::new(b"0123456789abcdeX", b"fedcba9876543210").unwrap();
        assert!(Manifest::decrypt(&ciphertext, &wrong).is_err());
    }

    #[test]
    fn rejects_short_keys() {
        assert!(ManifestKey::new(b"short", b"fedcba9876543210").is_err());
    }
}
