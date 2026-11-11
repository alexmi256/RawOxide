//! Camera reference profiles and format limits.
//!
//! Camera-profile model and helpers: the default Fujifilm GFX 100
//! combiner profile plus typed accessors for the harvested per-camera
//! registry under `assets/profiles/`. JSON parsing follows the
//! `CameraProfile.from_dict` fallback rules: missing calibration keys fall
//! back to the GFX reference values.

mod model;
mod parse;

pub use model::*;
