//! Output formats written by SekaiStoryRipper and read by SekaiStoryExporter.
//!
//! Every document carries a `format` name and a `version`. A reader must reject a
//! version it does not know instead of guessing.

pub mod episode;
pub mod motion;
pub mod path;
pub mod unpack;

pub use episode::EpisodeIndex;
pub use motion::SseMotion;
pub use unpack::UnpackRecord;
