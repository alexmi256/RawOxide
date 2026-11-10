//! Camera reference profiles and format limits.
//!
//! Mirrors Python `stegodng/profile.py`: the default Fujifilm GFX 100
//! combiner profile plus typed accessors for the harvested per-camera
//! registry (`stegodng/profiles/`). JSON parsing follows the Python
//! `CameraProfile.from_dict` fallback rules: missing calibration keys fall
//! back to the GFX reference values.

mod model;
mod parse;

pub use model::*;
