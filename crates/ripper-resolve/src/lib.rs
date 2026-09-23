//! Episode resolver: masterdata + scenario + manifest → the bundles one story episode needs, and
//! after unpacking, its `ripper-episode` index.
//!
//! Pure and synchronous; the CLI fetches masterdata and bundles and drives the steps:
//!
//! 1. [`catalog::Catalog`] lists the episodes of the masterdata, [`selector::Selector`] picks some;
//! 2. [`plan::scenario_bundle_candidates`] names the bundles that may hold the episode's
//!    scenario; the CLI fetches and unpacks them in order and takes the first one where
//!    [`index::find_scenario_file`] finds `<scenarioId>.json` (masterdata `scenarioId`);
//! 3. [`references::extract`] reads what the scenario refers to;
//! 4. [`plan::plan`] turns that into bundle names checked against the manifest (phase 1);
//! 5. after fetching and unpacking those, [`index::build_index`] looks every file up in the
//!    unpack records and writes the [`ripper_format::EpisodeIndex`] (phase 2).
//!
//! Bundle naming rules are in [`rules`] and [`live2d`] (reverse-engineered client).

pub mod catalog;
pub mod index;
pub mod live2d;
pub mod plan;
pub mod references;
pub mod rules;
pub mod selector;

pub use catalog::{Catalog, Episode, Masterdata};
pub use index::{IndexError, build_index};
pub use plan::{Manifest, Plan, PlanOptions, plan};
pub use references::{ScenarioRefs, extract};
pub use selector::Selector;
