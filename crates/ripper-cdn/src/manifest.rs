//! `AssetBundleInfoNew.json`: AES-128-CBC (ABCrypt) + PKCS#7, then msgpack.
//!
//! The ABCrypt key and IV are never shipped with this tool (ADR-0005); callers supply them.

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
    /// Parses a user-supplied key or IV: either the 16-character string itself or 32 hex digits.
    pub fn parse_part(text: &str) -> Result<[u8; 16], ManifestError> {
        let text = text.trim();
        if text.len() == 16 {
            return <[u8; 16]>::try_from(text.as_bytes()).map_err(|_| ManifestError::KeyLength {
                key: text.len(),
                iv: 0,
            });
        }
        if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut out = [0u8; 16];
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("checked hex");
            }
            return Ok(out);
        }
        Err(ManifestError::KeyLength {
            key: text.len(),
            iv: 0,
        })
    }

    pub fn from_text(key: &str, iv: &str) -> Result<Self, ManifestError> {
        Ok(Self {
            key: Self::parse_part(key)?,
            iv: Self::parse_part(iv)?,
        })
    }

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
    /// CDN directory holding this bundle (CN `ios1`, `ios10`, ...). The JP manifest has no such
    /// field; the store fills in `{assetVersion}/{assetHash}/{platform}` on load.
    #[serde(default)]
    pub download_path: String,
    /// CRC32 of the concatenated, decompressed bundle entries (the JP manifest's `crc` has the
    /// same meaning).
    pub crc: u32,
}

/// Decrypts `AssetBundleInfoNew.json` to its msgpack plaintext (what the manifest store archives).
pub fn decrypt(ciphertext: &[u8], key: &ManifestKey) -> Result<Vec<u8>, ManifestError> {
    Aes128CbcDec::new(&key.key.into(), &key.iv.into())
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|_| ManifestError::Decrypt)
}

/// The same envelope in the other direction (JP API request bodies).
pub(crate) fn encrypt(plain: &[u8], key: &ManifestKey) -> Vec<u8> {
    use aes::cipher::BlockModeEncrypt;
    cbc::Encryptor::<aes::Aes128>::new(&key.key.into(), &key.iv.into())
        .encrypt_padded_vec::<Pkcs7>(plain)
}

impl Manifest {
    pub fn decrypt(ciphertext: &[u8], key: &ManifestKey) -> Result<Self, ManifestError> {
        Self::from_msgpack(&decrypt(ciphertext, key)?)
    }

    pub fn from_msgpack(plain: &[u8]) -> Result<Self, ManifestError> {
        Ok(rmp_serde::from_slice(plain)?)
    }

    /// Gives every entry without a `downloadPath` (all of a JP manifest) the given one.
    pub fn fill_download_path(&mut self, download_path: &str) {
        for entry in self.bundles.values_mut() {
            if entry.download_path.is_empty() {
                entry.download_path = download_path.to_owned();
            }
        }
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
    fn parses_raw_and_hex_key_text() {
        let raw = ManifestKey::parse_part("0123456789abcdef").unwrap();
        assert_eq!(&raw, b"0123456789abcdef");
        let hex = ManifestKey::parse_part("30313233343536373839616263646566").unwrap();
        assert_eq!(hex, raw);
        assert!(ManifestKey::parse_part("0123").is_err());
    }

    #[test]
    fn rejects_short_keys() {
        assert!(ManifestKey::new(b"short", b"fedcba9876543210").is_err());
    }
}
