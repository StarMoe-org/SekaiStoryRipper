//! Output formats written by SekaiStoryRipper and read by SekaiStoryExporter.
//!
//! This crate is the whole interface between the two: a library (local directory or S3 prefix)
//! holds documents in these formats plus raw payloads (PNG, WAV, moc3, Live2D JSON, Unity
//! typetree JSON) whose schema is the game's. Every document carries a `format` name and a
//! `version`; `ripper.lock.json` lists them all ([`formats`]). A reader must reject a version it
//! does not know instead of guessing.

use std::collections::BTreeMap;

pub mod audio;
pub mod episode;
pub mod lock;
pub mod motion;
pub mod objects;
pub mod path;
pub mod unpack;

pub use episode::EpisodeIndex;
pub use lock::Lock;
pub use motion::SseMotion;
pub use objects::ObjectGraph;
pub use unpack::UnpackRecord;

/// Every format this crate describes, with the version it writes.
pub fn formats() -> BTreeMap<String, u32> {
    [
        (audio::FORMAT, audio::VERSION),
        (episode::FORMAT, episode::VERSION),
        (lock::FORMAT, lock::VERSION),
        (motion::FORMAT, motion::VERSION),
        (objects::FORMAT, objects::VERSION),
        (unpack::FORMAT, unpack::VERSION),
    ]
    .into_iter()
    .map(|(f, v)| (f.to_owned(), v))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_list_every_module() {
        let f = formats();
        assert_eq!(f.len(), 6);
        assert_eq!(f["ripper-objects"], 1);
        assert_eq!(f["ripper-unpack"], 3);
    }
}
