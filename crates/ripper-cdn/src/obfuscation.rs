//! Bundle obfuscation used by the CN CDN.
//!
//! A downloaded bundle is `fileSize + 4` bytes: a 4-byte header followed by a UnityFS file whose
//! first 128 bytes have the first 5 bytes of every 8-byte block inverted. A few files (for example
//! the ipa's `data.unity3d`) are plain UnityFS and are returned unchanged.

const HEADER_LEN: usize = 4;
const SCRAMBLED_PREFIX: usize = 128;
const BLOCK: usize = 8;
const INVERTED_PER_BLOCK: usize = 5;

/// Turns a downloaded bundle into plain UnityFS.
pub fn deobfuscate(mut data: Vec<u8>) -> Vec<u8> {
    if data.starts_with(b"UnityFS") {
        return data;
    }
    data.drain(..HEADER_LEN.min(data.len()));
    let scrambled = SCRAMBLED_PREFIX.min(data.len());
    for block in data[..scrambled].chunks_mut(BLOCK) {
        let inverted = INVERTED_PER_BLOCK.min(block.len());
        for byte in &mut block[..inverted] {
            *byte = !*byte;
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obfuscate(plain: &[u8]) -> Vec<u8> {
        let mut body = plain.to_vec();
        let scrambled = SCRAMBLED_PREFIX.min(body.len());
        for block in body[..scrambled].chunks_mut(BLOCK) {
            let inverted = INVERTED_PER_BLOCK.min(block.len());
            for byte in &mut block[..inverted] {
                *byte = !*byte;
            }
        }
        let mut out = vec![0x10, 0, 0, 0];
        out.extend(body);
        out
    }

    #[test]
    fn restores_the_unityfs_signature_and_leaves_the_tail_alone() {
        let mut plain = b"UnityFS\0".to_vec();
        plain.extend((0..300u32).map(|i| i as u8));
        let restored = deobfuscate(obfuscate(&plain));
        assert_eq!(restored, plain);
    }

    #[test]
    fn only_the_first_five_bytes_of_each_block_are_inverted() {
        let downloaded = [
            0x10, 0, 0, 0, 0xaa, 0x91, 0x96, 0x8b, 0x86, 0x46, 0x53, 0x00,
        ];
        assert_eq!(deobfuscate(downloaded.to_vec()), b"UnityFS\0");
    }

    #[test]
    fn plain_unityfs_is_passed_through() {
        let plain = b"UnityFS\0\0\0\0\x08".to_vec();
        assert_eq!(deobfuscate(plain.clone()), plain);
    }
}
