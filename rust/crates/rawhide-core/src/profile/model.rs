//! Profile model, reference defaults and fidelity policy.

use std::collections::BTreeMap;

use serde_json::Value;
use thiserror::Error;

use super::parse::{
    as_i64, as_u64, decode_hex, optional_usize, pair_or, pair_signed, pair_usize, parse_lenses,
    parse_matrix, parse_optional_float_pair, parse_optional_pair, parse_rational_list,
    parse_string_list, parse_subprofiles, parse_u16_list, parse_u32_list, parse_version, rationals,
    string_or, strip_nulls, u8_or, u16_or, u32_or,
};

/// Maximum writable LSB planes.
pub const MAX_LSB_PLANES: u8 = 16;

/// Maximum full-resolution raw frames in one file.
pub const MAX_FRAMES: u8 = 8;

/// Integer sample depths with TIFF/DNG `BitsPerSample` support.
pub const SUPPORTED_BIT_DEPTHS: [u8; 5] = [8, 10, 12, 14, 16];

/// GFX 100 II native sensor width.
pub const GFX_NATIVE_W: usize = 11_648;

/// GFX 100 II native sensor height.
pub const GFX_NATIVE_H: usize = 8_736;

/// Pixel Shift Combiner output width.
pub const PIXELSHIFT_W: usize = 23_296;

/// Pixel Shift Combiner output height.
pub const PIXELSHIFT_H: usize = 17_472;

/// Floor for `--auto-size` width: JPEG-MCU-friendly and Bayer-safe.
pub const AUTO_MIN_W: usize = 64;

/// Floor for `--auto-size` height.
pub const AUTO_MIN_H: usize = 48;

/// Preset geometries for auto-configuration, smallest first.
pub const PRESET_GEOMETRIES: [(usize, usize, &str); 4] = [
    (2_048, 1_536, "demo"),
    (4_000, 3_000, "combiner-scale"),
    (GFX_NATIVE_W, GFX_NATIVE_H, "gfx-native"),
    (PIXELSHIFT_W, PIXELSHIFT_H, "pixelshift"),
];

/// Reference `OpcodeList3` copied from the GFX combiner sample.
const DEFAULT_OPCODE_LIST3_HEX: &str = concat!(
    "00000002000000010103000000000001000000a4000000033fefefa6115f8d8b",
    "bd475000000000003d41000000000000bd3dc000000000000000000000000000",
    "00000000000000003fefefa6115f8d8bbd475000000000003d41000000000000",
    "bd3dc00000000000000000000000000000000000000000003fefefa6115f8d8b",
    "bd475000000000003d41000000000000bd3dc000000000000000000000000000",
    "00000000000000003fe00000000000003fe00000000000000000000301030000",
    "00000001000000383fad0f10239baaa43fb463dcfc19bda0bfd51fa03a2241f0",
    "3fdbc76f7c9e4ad0bfc809334b6932d03fe00000000000003fe0000000000000",
);

/// One lens pool entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Lens {
    /// Model name.
    pub model: String,
    /// Minimum focal length in mm.
    pub min_mm: f64,
    /// Maximum focal length in mm.
    pub max_mm: f64,
    /// Maximum aperture.
    pub max_aperture: f64,
    /// Lens make override.
    pub make: Option<String>,
}

/// Per-profile `GainMap` base grid and jitter settings.
#[derive(Debug, Clone, PartialEq)]
pub struct GainMapConfig {
    /// Base grid rows.
    pub rows: usize,
    /// Base grid columns.
    pub cols: usize,
    /// Four `rows * cols` planes in CFA phase order (B, G, G, R).
    pub planes: Vec<f64>,
    /// Per-file jitter.
    pub jitter: f64,
    /// Gain clip range.
    pub clip: (f64, f64),
}

impl GainMapConfig {
    /// Parse the `gainmap` block.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for malformed grids or values.
    pub fn from_json(data: &Value) -> Result<Self, ProfileError> {
        let object = strip_nulls(data.clone());
        let object = object
            .as_object()
            .ok_or_else(|| ProfileError::Invalid("gainmap must be a dict".to_owned()))?;
        let planes = object
            .get("planes")
            .and_then(Value::as_array)
            .ok_or_else(|| ProfileError::Invalid("gainmap needs a planes grid".to_owned()))?;
        if planes.len() != 4 {
            return Err(ProfileError::Invalid(
                "gainmap planes must hold four grids".to_owned(),
            ));
        }
        let mut rows = None;
        let mut cols = None;
        let mut flat = Vec::new();
        for plane in planes {
            let rows_value = plane.as_array().ok_or_else(|| {
                ProfileError::Invalid("gainmap planes must be nested lists".to_owned())
            })?;
            let plane_cols = rows_value
                .first()
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if rows.is_some_and(|value| value != rows_value.len())
                || cols.is_some_and(|value| value != plane_cols)
                || rows_value.len() < 2
                || plane_cols < 2
            {
                return Err(ProfileError::Invalid(
                    "gainmap planes must be a rectangular 4 x V x H grid".to_owned(),
                ));
            }
            rows = Some(rows_value.len());
            cols = Some(plane_cols);
            for row in rows_value {
                let row = row
                    .as_array()
                    .filter(|row| row.len() == plane_cols)
                    .ok_or_else(|| {
                        ProfileError::Invalid("gainmap grid rows must match".to_owned())
                    })?;
                for value in row {
                    let gain = value.as_f64().ok_or_else(|| {
                        ProfileError::Invalid("gainmap values must be numbers".to_owned())
                    })?;
                    if !gain.is_finite() || gain <= 0.0 {
                        return Err(ProfileError::Invalid(
                            "gainmap base values must be finite and positive".to_owned(),
                        ));
                    }
                    flat.push(gain);
                }
            }
        }
        if let Some(grid) = object.get("grid") {
            let values = grid
                .as_array()
                .filter(|values| values.len() == 2)
                .ok_or_else(|| {
                    ProfileError::Invalid("gainmap grid must be [rows, cols]".to_owned())
                })?;
            let grid_rows = usize::try_from(as_u64(&values[0], "gainmap.grid")?).unwrap_or(0);
            let grid_cols = usize::try_from(as_u64(&values[1], "gainmap.grid")?).unwrap_or(0);
            if grid_rows < 2 || grid_cols < 2 {
                return Err(ProfileError::Invalid(
                    "gainmap grid must be [rows, cols] >= 2".to_owned(),
                ));
            }
            if grid_rows != rows.unwrap_or(0) || grid_cols != cols.unwrap_or(0) {
                return Err(ProfileError::Invalid(
                    "gainmap planes shape does not match grid".to_owned(),
                ));
            }
        }
        let clip = object.get("clip").map_or(Ok((1.0, 4.2)), |clip| {
            let values = clip
                .as_array()
                .filter(|values| values.len() == 2)
                .ok_or_else(|| {
                    ProfileError::Invalid("gainmap clip must be [low, high]".to_owned())
                })?;
            let low = values[0].as_f64().unwrap_or(0.0);
            let high = values[1].as_f64().unwrap_or(0.0);
            if !(low.is_finite() && high.is_finite() && 0.0 < low && low < high) {
                return Err(ProfileError::Invalid(
                    "gainmap clip must be two increasing positives".to_owned(),
                ));
            }
            Ok((low, high))
        })?;
        let jitter = object
            .get("jitter")
            .and_then(Value::as_f64)
            .unwrap_or(0.025);
        if !jitter.is_finite() || jitter < 0.0 {
            return Err(ProfileError::Invalid(
                "gainmap jitter must be a non-negative number".to_owned(),
            ));
        }
        Ok(Self {
            rows: rows.unwrap_or(0),
            cols: cols.unwrap_or(0),
            planes: flat,
            jitter,
            clip,
        })
    }
}

/// Adobe camera-profile tags/blobs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AdobeProfile {
    /// Forward matrix 1.
    pub forward_matrix1: Option<[(i32, i32); 9]>,
    /// Forward matrix 2.
    pub forward_matrix2: Option<[(i32, i32); 9]>,
    /// Profile name.
    pub name: Option<String>,
    /// Calibration signature.
    pub calibration_signature: Option<String>,
    /// Copyright string.
    pub copyright: Option<String>,
    /// Embed policy.
    pub embed_policy: Option<u32>,
    /// Hue/Sat map dimensions.
    pub hue_sat_map_dims: Option<[u32; 3]>,
    /// Hue/Sat map plane 1.
    pub hue_sat_map_data1: Option<Vec<f32>>,
    /// Hue/Sat map plane 2.
    pub hue_sat_map_data2: Option<Vec<f32>>,
    /// Look-table dimensions.
    pub look_table_dims: Option<[u32; 3]>,
    /// Look-table data.
    pub look_table_data: Option<Vec<f32>>,
}

impl AdobeProfile {
    fn from_json(
        data: &Value,
        assets: &dyn Fn(&str) -> Option<&'static [u8]>,
    ) -> Result<Self, ProfileError> {
        let object = strip_nulls(data.clone());
        let object = object
            .as_object()
            .ok_or_else(|| ProfileError::Invalid("fidelity.adobe must be a dict".to_owned()))?;
        let matrix = |value: Option<&Value>| -> Result<Option<[(i32, i32); 9]>, ProfileError> {
            let Some(value) = value else { return Ok(None) };
            let items = value
                .as_array()
                .filter(|items| items.len() == 9)
                .ok_or_else(|| {
                    ProfileError::Invalid("adobe matrix needs nine rationals".to_owned())
                })?;
            let mut out = [(0, 0); 9];
            for (slot, item) in out.iter_mut().zip(items) {
                let pair = pair_signed(item, "adobe.forward_matrix")?;
                if pair.1 <= 0 {
                    return Err(ProfileError::Invalid(
                        "adobe matrix denominators must be positive".to_owned(),
                    ));
                }
                *slot = pair;
            }
            Ok(Some(out))
        };
        let dims = |value: Option<&Value>, name: &str| -> Result<Option<[u32; 3]>, ProfileError> {
            let Some(value) = value else { return Ok(None) };
            let items = value
                .as_array()
                .filter(|items| items.len() == 3)
                .ok_or_else(|| ProfileError::Invalid(format!("adobe.{name} needs three values")))?;
            Ok(Some([
                u32::try_from(as_u64(&items[0], name)?).unwrap_or(0),
                u32::try_from(as_u64(&items[1], name)?).unwrap_or(0),
                u32::try_from(as_u64(&items[2], name)?).unwrap_or(0),
            ]))
        };
        let floats = |name: Option<&str>,
                      expected: Option<usize>|
         -> Result<Option<Vec<f32>>, ProfileError> {
            let Some(name) = name else { return Ok(None) };
            let bytes = assets(name)
                .ok_or_else(|| ProfileError::Invalid(format!("cannot read adobe asset {name}")))?;
            if let Some(expected) = expected
                && bytes.len() != expected
            {
                return Err(ProfileError::Invalid(format!(
                    "adobe asset {name} is {} bytes, expected {expected}",
                    bytes.len()
                )));
            }
            if !bytes.len().is_multiple_of(4) {
                return Err(ProfileError::Invalid(format!(
                    "adobe asset {name} is not float data"
                )));
            }
            Ok(Some(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect(),
            ))
        };
        let hue_dims = dims(object.get("hue_sat_map_dims"), "hue_sat_map_dims")?;
        let look_dims = dims(object.get("look_table_dims"), "look_table_dims")?;
        let hue_expected = hue_dims.map(|dims| {
            (dims[0] as usize)
                .saturating_mul(dims[1] as usize)
                .saturating_mul(dims[2] as usize)
                .saturating_mul(3)
                .saturating_mul(4)
        });
        let look_expected = look_dims.map(|dims| {
            (dims[0] as usize)
                .saturating_mul(dims[1] as usize)
                .saturating_mul(dims[2] as usize)
                .saturating_mul(3)
                .saturating_mul(4)
        });
        Ok(Self {
            forward_matrix1: matrix(object.get("forward_matrix1"))?,
            forward_matrix2: matrix(object.get("forward_matrix2"))?,
            name: object
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            calibration_signature: object
                .get("calibration_signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
            copyright: object
                .get("copyright")
                .and_then(Value::as_str)
                .map(str::to_owned),
            embed_policy: object
                .get("embed_policy")
                .and_then(Value::as_u64)
                .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            hue_sat_map_dims: hue_dims,
            hue_sat_map_data1: floats(
                object.get("hue_sat_map_data1").and_then(Value::as_str),
                hue_expected,
            )?,
            hue_sat_map_data2: floats(
                object.get("hue_sat_map_data2").and_then(Value::as_str),
                hue_expected,
            )?,
            look_table_dims: look_dims,
            look_table_data: floats(
                object.get("look_table_data").and_then(Value::as_str),
                look_expected,
            )?,
        })
    }
}

/// Envelope of one camera's opaque `MakerNote` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakernoteConfig {
    /// Magic bytes (usually `HDRP`).
    pub magic: Vec<u8>,
    /// Constant prefix after the magic.
    pub prefix: Vec<u8>,
    /// Inclusive total-size range.
    pub length: (usize, usize),
}

impl MakernoteConfig {
    /// Parse the fidelity `makernote` block.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for malformed magic/prefix/length.
    pub fn from_json(data: &Value) -> Result<Self, ProfileError> {
        let object = strip_nulls(data.clone());
        let object = object
            .as_object()
            .ok_or_else(|| ProfileError::Invalid("makernote must be a dict".to_owned()))?;
        let magic = object
            .get("magic")
            .and_then(Value::as_str)
            .unwrap_or("HDRP")
            .as_bytes()
            .to_vec();
        if magic.is_empty() {
            return Err(ProfileError::Invalid(
                "makernote.magic must not be empty".to_owned(),
            ));
        }
        let prefix = match object.get("prefix_hex").and_then(Value::as_str) {
            Some(hex) => {
                let cleaned: String = hex.chars().filter(|ch| !ch.is_whitespace()).collect();
                decode_hex(&cleaned).ok_or_else(|| {
                    ProfileError::Invalid("makernote.prefix_hex must be valid hex".to_owned())
                })?
            }
            None => Vec::new(),
        };
        let (low, high) = match object.get("length") {
            Some(length) => {
                let values = length
                    .as_array()
                    .filter(|values| values.len() == 2)
                    .ok_or_else(|| {
                        ProfileError::Invalid("makernote.length must be [low, high]".to_owned())
                    })?;
                (
                    usize::try_from(as_u64(&values[0], "makernote.length")?).unwrap_or(0),
                    usize::try_from(as_u64(&values[1], "makernote.length")?).unwrap_or(0),
                )
            }
            None => (2640, 2757),
        };
        if low > high || low == 0 {
            return Err(ProfileError::Invalid(
                "makernote.length must be low <= high, low >= 1".to_owned(),
            ));
        }
        if magic.len() + prefix.len() > low {
            return Err(ProfileError::Invalid(
                "makernote prefix is longer than the shortest allowed block".to_owned(),
            ));
        }
        Ok(Self {
            magic,
            prefix,
            length: (low, high),
        })
    }

    /// Generate a deterministic block using the caller's RNG.
    #[must_use]
    pub fn generate(&self, rng: &mut crate::metadata::Rng64) -> Vec<u8> {
        let span = self.length.1.saturating_sub(self.length.0);
        let total = self.length.0 + rng.below(u64::try_from(span).unwrap_or(0) + 1) as usize;
        let body_len = total.saturating_sub(self.magic.len() + self.prefix.len());
        let mut block = Vec::with_capacity(total);
        block.extend_from_slice(&self.magic);
        block.extend_from_slice(&self.prefix);
        for _ in 0..body_len {
            block.push(rng.below(256) as u8);
        }
        block
    }
}

/// Optional metadata/raw fidelity overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct FidelityConfig {
    /// Preferred preview size.
    pub preview_size: Option<(usize, usize)>,
    /// Allowed EXIF orientation values.
    pub orientations: Vec<u16>,
    /// Software strings to draw from.
    pub software_variants: Vec<String>,
    /// Inclusive ISO date range.
    pub date_range: Option<(String, String)>,
    /// Window ending now, in days.
    pub date_window_days: Option<u32>,
    /// Suggested raw mode.
    pub suggested_mode: Option<String>,
    /// `AsShotNeutral` red range (x10000).
    pub asn_r: Option<(i32, i32)>,
    /// `AsShotNeutral` blue range (x10000).
    pub asn_b: Option<(i32, i32)>,
    /// `BaselineExposure` range (`EV` x10000).
    pub baseline_exposure: Option<(i32, i32)>,
    /// Subject distance choices.
    pub subject_distance: Vec<(u32, u32)>,
    /// Subject distance range choices.
    pub subject_distance_range: Vec<u16>,
    /// Exposure bias choices (EV x100).
    pub exposure_bias: Vec<i32>,
    /// Flash tag value.
    pub flash: Option<u16>,
    /// Emit XMP.
    pub emit_xmp: bool,
    /// Emit lens tags.
    pub emit_lens: bool,
    /// Emit serial tags.
    pub emit_serials: bool,
    /// Emit GPS.
    pub emit_gps: bool,
    /// EXIF tag allow-list.
    pub exif_allow: Option<Vec<u32>>,
    /// Tag convention (`adobe`).
    pub tag_conventions: Option<String>,
    /// Emit raw digests.
    pub raw_digests: bool,
    /// Opaque `MakerNote` envelope.
    pub makernote: Option<MakernoteConfig>,
    /// Sub-second fraction digits.
    pub subsec_digits: u8,
    /// Raw tile geometry as `(tile_length, tile_width)` (tifffile order).
    pub tile: Option<(usize, usize)>,
    /// Per-channel black levels.
    pub black_levels: Option<Vec<(u32, u32)>>,
    /// Effective white level.
    pub white_level: Option<u32>,
    /// Plane ceiling.
    pub max_planes: Option<u8>,
    /// Level policy (`strict`/`capacity`).
    pub level_policy: String,
    /// Crop margin.
    pub crop_margin: Option<(usize, usize)>,
    /// Bayer green split.
    pub bayer_green_split: Option<u16>,
    /// Sensor geometries.
    pub sensor_sizes: Vec<(usize, usize)>,
    /// Prefer native sensor geometry.
    pub prefer_native: bool,
    /// Adobe profile tags/assets.
    pub adobe: Option<AdobeProfile>,
    /// Read noise range.
    pub noise_s: Option<(f64, f64)>,
    /// Offset noise range.
    pub noise_o: Option<(f64, f64)>,
}

impl Default for FidelityConfig {
    fn default() -> Self {
        Self {
            preview_size: None,
            orientations: Vec::new(),
            software_variants: Vec::new(),
            date_range: None,
            date_window_days: None,
            suggested_mode: None,
            asn_r: None,
            asn_b: None,
            baseline_exposure: None,
            subject_distance: Vec::new(),
            subject_distance_range: Vec::new(),
            exposure_bias: Vec::new(),
            flash: None,
            emit_xmp: true,
            emit_lens: true,
            emit_serials: true,
            emit_gps: false,
            exif_allow: None,
            tag_conventions: None,
            raw_digests: false,
            makernote: None,
            subsec_digits: 2,
            tile: None,
            black_levels: None,
            white_level: None,
            max_planes: None,
            level_policy: "capacity".to_owned(),
            crop_margin: None,
            bayer_green_split: None,
            sensor_sizes: Vec::new(),
            prefer_native: false,
            adobe: None,
            noise_s: None,
            noise_o: None,
        }
    }
}

impl FidelityConfig {
    /// Parse the `fidelity` block from a registry profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for malformed ranges or values.
    pub fn from_json(
        data: &Value,
        assets: &dyn Fn(&str) -> Option<&'static [u8]>,
    ) -> Result<Self, ProfileError> {
        let data = strip_nulls(data.clone());
        let object = data
            .as_object()
            .ok_or_else(|| ProfileError::Invalid("fidelity must be a dict".to_owned()))?;
        let mut out = Self::default();

        if let Some(preview) = object.get("preview") {
            let size = preview
                .get("size")
                .ok_or_else(|| ProfileError::Invalid("preview.size is missing".to_owned()))?;
            let (width, height) = pair_usize(size, "preview.size")?;
            if width == 0 || height == 0 {
                return Err(ProfileError::Invalid(
                    "preview.size values must be positive".to_owned(),
                ));
            }
            out.preview_size = Some((width, height));
        }
        if let Some(values) = object.get("orientation").and_then(Value::as_array) {
            for value in values {
                let orientation = as_u64(value, "orientation")?;
                if !(1..=8).contains(&orientation) {
                    return Err(ProfileError::Invalid(
                        "orientation values must be 1..8".to_owned(),
                    ));
                }
                out.orientations
                    .push(u16::try_from(orientation).unwrap_or(1));
            }
        }
        if let Some(values) = object.get("software_variants").and_then(Value::as_array) {
            out.software_variants = values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
        }
        if let Some(range) = object.get("date_range") {
            let array = range
                .as_array()
                .filter(|items| items.len() == 2)
                .ok_or_else(|| {
                    ProfileError::Invalid("date_range must be [start, end]".to_owned())
                })?;
            let start = array[0]
                .as_str()
                .ok_or_else(|| ProfileError::Invalid("date_range must be strings".to_owned()))?;
            let end = array[1]
                .as_str()
                .ok_or_else(|| ProfileError::Invalid("date_range must be strings".to_owned()))?;
            chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(|_| {
                ProfileError::Invalid(format!("date_range values must be YYYY-MM-DD: {start:?}"))
            })?;
            chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").map_err(|_| {
                ProfileError::Invalid(format!("date_range values must be YYYY-MM-DD: {end:?}"))
            })?;
            out.date_range = Some((start.to_owned(), end.to_owned()));
        }
        if let Some(days) = object.get("date_window_days").and_then(Value::as_u64) {
            if !(1..=3650).contains(&days) {
                return Err(ProfileError::Invalid(
                    "date_window_days must be 1..3650".to_owned(),
                ));
            }
            out.date_window_days = Some(u32::try_from(days).unwrap_or(365));
        }
        if let Some(mode) = object.get("suggested_mode") {
            let mode = mode.as_str().unwrap_or_default();
            if mode != "linear" && mode != "cfa" {
                return Err(ProfileError::Invalid(
                    "suggested_mode must be linear or cfa".to_owned(),
                ));
            }
            out.suggested_mode = Some(mode.to_owned());
        }
        if let Some(asn) = object.get("as_shot_neutral") {
            out.asn_r = parse_optional_pair(asn.get("r"), "as_shot_neutral.r")?;
            out.asn_b = parse_optional_pair(asn.get("b"), "as_shot_neutral.b")?;
            for (name, pair) in [
                ("as_shot_neutral.r", out.asn_r),
                ("as_shot_neutral.b", out.asn_b),
            ] {
                if let Some((low, high)) = pair
                    && !(0 < low && low <= high && high <= 10_000)
                {
                    return Err(ProfileError::Invalid(format!(
                        "{name} must be 0 < low <= high <= 10000"
                    )));
                }
            }
        }
        if let Some(baseline) = object.get("baseline_exposure") {
            let pair = pair_signed(baseline, "baseline_exposure")?;
            if !(-10_000 <= pair.0 && pair.0 <= pair.1 && pair.1 <= 10_000) {
                return Err(ProfileError::Invalid(
                    "baseline_exposure must be within -1..+1 EV".to_owned(),
                ));
            }
            out.baseline_exposure = Some(pair);
        }
        if let Some(subject) = object.get("subject_distance") {
            if let Some(choices) = subject.get("choices") {
                out.subject_distance = rationals(choices, "subject_distance.choices")?;
            }
            if let Some(range) = subject.get("range").and_then(Value::as_array) {
                out.subject_distance_range = range
                    .iter()
                    .map(|value| {
                        as_u64(value, "subject_distance.range")
                            .map(|item| u16::try_from(item).unwrap_or(u16::MAX))
                    })
                    .collect::<Result<_, _>>()?;
            }
        }
        if let Some(tags) = object.get("tags") {
            out.emit_xmp = tags.get("xmp").and_then(Value::as_bool).unwrap_or(true);
            out.emit_lens = tags.get("lens").and_then(Value::as_bool).unwrap_or(true);
            out.emit_serials = tags.get("serials").and_then(Value::as_bool).unwrap_or(true);
            out.emit_gps = tags.get("gps").and_then(Value::as_bool).unwrap_or(false);
            if let Some(allow) = tags.get("exif_allow").and_then(Value::as_array) {
                out.exif_allow = Some(
                    allow
                        .iter()
                        .map(|value| {
                            as_u64(value, "tags.exif_allow")
                                .map(|item| u32::try_from(item).unwrap_or(u32::MAX))
                        })
                        .collect::<Result<_, _>>()?,
                );
            }
            if let Some(bias) = tags.get("exposure_bias").and_then(Value::as_array) {
                out.exposure_bias = bias
                    .iter()
                    .map(|value| {
                        as_i64(value, "tags.exposure_bias")
                            .map(|item| i32::try_from(item).unwrap_or(0))
                    })
                    .collect::<Result<_, _>>()?;
            }
            if let Some(flash) = tags.get("flash").and_then(Value::as_u64) {
                out.flash = Some(u16::try_from(flash).unwrap_or(u16::MAX));
            }
        }
        if let Some(conventions) = object.get("tag_conventions") {
            let conventions = conventions.as_str().unwrap_or_default();
            if conventions != "adobe" {
                return Err(ProfileError::Invalid(
                    "tag_conventions must be one of ('adobe',)".to_owned(),
                ));
            }
            out.tag_conventions = Some(conventions.to_owned());
        }
        out.raw_digests = object
            .get("raw_digests")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(makernote) = object.get("makernote") {
            out.makernote = Some(MakernoteConfig::from_json(makernote)?);
        }
        out.subsec_digits = object
            .get("subsec_digits")
            .and_then(Value::as_u64)
            .map_or_else(
                || {
                    if out.tag_conventions.as_deref() == Some("adobe") {
                        6
                    } else {
                        2
                    }
                },
                |digits| u8::try_from(digits).unwrap_or(2),
            );
        if !(1..=9).contains(&out.subsec_digits) {
            return Err(ProfileError::Invalid(
                "subsec_digits must be 1..9".to_owned(),
            ));
        }
        if let Some(raw) = object.get("raw") {
            if raw.get("tile").is_some() {
                out.tile = Some(pair_usize(&raw["tile"], "raw.tile")?);
            }
            if raw.get("black_levels").is_some() {
                let black = rationals(&raw["black_levels"], "raw.black_levels")?;
                if !matches!(black.len(), 1 | 3 | 4) {
                    return Err(ProfileError::Invalid(
                        "raw.black_levels needs 1, 3 or 4 values".to_owned(),
                    ));
                }
                out.black_levels = Some(black);
            }
            if let Some(white) = raw.get("white_level").and_then(Value::as_u64) {
                if !(1..=65_535).contains(&white) {
                    return Err(ProfileError::Invalid(
                        "raw.white_level must be 1..65535".to_owned(),
                    ));
                }
                out.white_level = Some(u32::try_from(white).unwrap_or(65_535));
            }
            if let Some(planes) = raw.get("max_planes").and_then(Value::as_u64) {
                if !(1..=16).contains(&planes) {
                    return Err(ProfileError::Invalid(
                        "raw.max_planes must be 1..16".to_owned(),
                    ));
                }
                out.max_planes = Some(u8::try_from(planes).unwrap_or(16));
            }
            if let Some(policy) = raw.get("level_policy").and_then(Value::as_str) {
                let policy = policy.to_lowercase();
                if policy != "strict" && policy != "capacity" {
                    return Err(ProfileError::Invalid(
                        "raw.level_policy must be strict or capacity".to_owned(),
                    ));
                }
                out.level_policy = policy;
            }
            if raw.get("crop_margin").is_some() {
                out.crop_margin = Some(pair_usize(&raw["crop_margin"], "raw.crop_margin")?);
            }
            if let Some(green) = raw.get("bayer_green_split").and_then(Value::as_u64) {
                out.bayer_green_split = Some(u16::try_from(green).unwrap_or(u16::MAX));
            }
        }
        if let Some(geometry) = object.get("geometry") {
            if let Some(sizes) = geometry.get("sensor_sizes").and_then(Value::as_array) {
                for size in sizes {
                    out.sensor_sizes
                        .push(pair_usize(size, "geometry.sensor_sizes")?);
                }
            }
            out.prefer_native = geometry
                .get("prefer_native")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }
        if let Some(noise) = object.get("noise") {
            out.noise_s = parse_optional_float_pair(noise.get("s"), "noise.s")?;
            out.noise_o = parse_optional_float_pair(noise.get("o"), "noise.o")?;
        }
        if let Some(adobe) = object.get("adobe") {
            out.adobe = Some(AdobeProfile::from_json(adobe, assets)?);
        }
        Ok(out)
    }

    /// Capture date bounds; `date_window_days` ends now.
    #[must_use]
    pub fn date_bounds(&self) -> Option<(chrono::NaiveDateTime, chrono::NaiveDateTime)> {
        self.date_bounds_at(chrono::Local::now().naive_local())
    }

    /// Capture date bounds with an explicit end for the rolling window.
    ///
    /// Encoding passes one anchor per run so every chunk of a split set
    /// shares one window and the seeded metadata does not shift when a run
    /// crosses a second boundary.
    #[must_use]
    pub fn date_bounds_at(
        &self,
        end: chrono::NaiveDateTime,
    ) -> Option<(chrono::NaiveDateTime, chrono::NaiveDateTime)> {
        use chrono::Duration;
        if let Some(days) = self.date_window_days {
            return Some((end - Duration::days(i64::from(days)), end));
        }
        let (start, end_date) = self.date_range.as_ref()?;
        let start = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
        let end_date = chrono::NaiveDate::parse_from_str(end_date, "%Y-%m-%d").ok()?;
        let start = start.and_hms_opt(0, 0, 0)?;
        let end = (end_date + Duration::days(1)).and_hms_opt(0, 0, 0)? - Duration::seconds(1);
        Some((start, end))
    }
}

/// Static reference values for one camera/cover story.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraProfile {
    /// Camera make.
    pub make: String,
    /// Camera model.
    pub model: String,
    /// Unique camera model.
    pub unique_camera_model: String,
    /// Software string.
    pub software: String,
    /// DNG version bytes.
    pub dng_version: [u8; 4],
    /// Backward version bytes.
    pub dng_backward: [u8; 4],
    /// Color matrix 1.
    pub color_matrix1: [(i32, u32); 9],
    /// Color matrix 2.
    pub color_matrix2: [(i32, u32); 9],
    /// Calibration illuminant 1.
    pub calib_illuminant1: u16,
    /// Calibration illuminant 2.
    pub calib_illuminant2: u16,
    /// Baseline noise.
    pub baseline_noise: (u32, u32),
    /// Baseline sharpness.
    pub baseline_sharpness: (u32, u32),
    /// Linear response limit.
    pub linear_response_limit: (u32, u32),
    /// Shadow scale.
    pub shadow_scale: (u32, u32),
    /// Combiner reference black level (256 at 16-bit).
    pub black_level_ref: u32,
    /// `OpcodeList3` bytes.
    pub opcode_list3: Vec<u8>,
    /// Metadata fidelity overlay.
    pub fidelity: Option<FidelityConfig>,
    /// Optional `GainMap` base grid.
    pub gainmap: Option<GainMapConfig>,
    /// CLI slug.
    pub slug: String,
    /// Human-readable name.
    pub display_name: String,
    /// Native width.
    pub native_width: Option<usize>,
    /// Native height.
    pub native_height: Option<usize>,
    /// Suggested depth.
    pub suggested_bit_depth: u8,
    /// CFA pattern.
    pub cfa_pattern: Option<String>,
    /// Crop factor.
    pub crop_factor: f64,
    /// Lens pool.
    pub lenses: Vec<Lens>,
    /// ISO pool.
    pub iso_choices: Vec<u32>,
    /// Exposure pool.
    pub exposure_choices: Vec<(u32, u32)>,
    /// F-number pool (x100).
    pub fnumber_choices: Vec<(u32, u32)>,
    /// Metering pool.
    pub metering_choices: Vec<u16>,
    /// Exposure program pool.
    pub exposure_program_choices: Vec<u16>,
    /// Serial prefixes.
    pub serial_prefixes: Vec<String>,
    /// Subprofile geometries.
    pub subprofiles: BTreeMap<String, (usize, usize)>,
    /// Calibration provenance.
    pub calibration_source: String,
}

impl Default for CameraProfile {
    fn default() -> Self {
        Self {
            make: "FUJIFILM".to_owned(),
            model: "GFX 100".to_owned(),
            unique_camera_model: "GFX 100".to_owned(),
            software: "FUJIFILM Pixel Shift Combiner".to_owned(),
            dng_version: [1, 4, 0, 0],
            dng_backward: [1, 1, 0, 0],
            color_matrix1: [
                (17_191, 10_000),
                (-11_000, 10_000),
                (1_278, 10_000),
                (-3_574, 10_000),
                (11_733, 10_000),
                (2_076, 10_000),
                (-2, 10_000),
                (497, 10_000),
                (6_540, 10_000),
            ],
            color_matrix2: [
                (16_212, 10_000),
                (-8_423, 10_000),
                (-1_583, 10_000),
                (-4_336, 10_000),
                (12_583, 10_000),
                (1_937, 10_000),
                (-195, 10_000),
                (726, 10_000),
                (6_199, 10_000),
            ],
            calib_illuminant1: 17,
            calib_illuminant2: 21,
            baseline_noise: (10_000, 10_000),
            baseline_sharpness: (13_300, 10_000),
            linear_response_limit: (10_000, 10_000),
            shadow_scale: (10_000, 10_000),
            black_level_ref: 256,
            opcode_list3: decode_hex(DEFAULT_OPCODE_LIST3_HEX).unwrap_or_default(),
            fidelity: None,
            gainmap: None,
            slug: "gfx_100".to_owned(),
            display_name: "Fujifilm GFX 100".to_owned(),
            native_width: Some(GFX_NATIVE_W),
            native_height: Some(GFX_NATIVE_H),
            suggested_bit_depth: 16,
            cfa_pattern: None,
            crop_factor: 0.79,
            lenses: Vec::new(),
            iso_choices: Vec::new(),
            exposure_choices: Vec::new(),
            fnumber_choices: Vec::new(),
            metering_choices: Vec::new(),
            exposure_program_choices: Vec::new(),
            serial_prefixes: ["92A", "94A", "93A", "95A"]
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            subprofiles: BTreeMap::from([
                ("native".to_owned(), (GFX_NATIVE_W, GFX_NATIVE_H)),
                ("pixelshift".to_owned(), (PIXELSHIFT_W, PIXELSHIFT_H)),
            ]),
            calibration_source: "reference-gfx-combiner".to_owned(),
        }
    }
}

impl CameraProfile {
    /// White level for a depth.
    #[must_use]
    pub const fn white_level(bit_depth: u8) -> u32 {
        (1_u32 << bit_depth) - 1
    }

    /// Scaled black level.
    #[must_use]
    pub fn black_level(&self, bit_depth: u8) -> u32 {
        let peak = Self::white_level(bit_depth);
        let value = (u64::from(self.black_level_ref) * u64::from(peak) + 32_767) / 65_535;
        u32::try_from(value).unwrap_or(u32::MAX).max(1)
    }

    /// Storage width: 8-bit fits `u8`, everything else uses `u16`.
    #[must_use]
    pub const fn container_bits(bit_depth: u8) -> u8 {
        if bit_depth <= 8 { 8 } else { 16 }
    }

    /// Highest writable plane count.
    #[must_use]
    pub const fn top_planes(bit_depth: u8) -> u8 {
        if bit_depth < MAX_LSB_PLANES {
            bit_depth
        } else {
            MAX_LSB_PLANES
        }
    }

    /// White level after the fidelity policy.
    #[must_use]
    pub fn effective_white(&self, bit_depth: u8, policy: &str) -> u32 {
        let peak = Self::white_level(bit_depth);
        let Some(fidelity) = &self.fidelity else {
            return peak;
        };
        let Some(white) = fidelity.white_level else {
            return peak;
        };
        if self.policy(policy) == "capacity" {
            peak
        } else {
            peak.min(white)
        }
    }

    /// Effective plane ceiling after the fidelity policy.
    #[must_use]
    pub fn effective_max_planes(&self, bit_depth: u8, policy: &str) -> u8 {
        let mut top = Self::top_planes(bit_depth);
        let Some(fidelity) = &self.fidelity else {
            return top;
        };
        if self.policy(policy) != "strict" {
            return top;
        }
        if let Some(planes) = fidelity.max_planes {
            top = top.min(planes);
        }
        let white = self
            .effective_white(bit_depth, "strict")
            .min(Self::white_level(bit_depth));
        let safe = white.saturating_add(1).ilog2().max(1);
        u8::try_from(safe).unwrap_or(MAX_LSB_PLANES).min(top)
    }

    /// Raw tile `(width, height)` requested by the profile fidelity block.
    ///
    /// The profile JSON stores tifffile's `(tile_length, tile_width)` order;
    /// this helper swaps it into the writer's width/height order.
    #[must_use]
    pub fn raw_tile_wh(&self) -> Option<(usize, usize)> {
        self.fidelity
            .as_ref()
            .and_then(|fidelity| fidelity.tile)
            .map(|(tile_length, tile_width)| (tile_width, tile_length))
    }

    /// Per-channel black levels scaled to the effective white.
    #[must_use]
    pub fn effective_black_levels(&self, mode: &str, bit_depth: u8, policy: &str) -> Vec<u32> {
        let Some(fidelity) = &self.fidelity else {
            let count = if mode == "linear" { 3 } else { 1 };
            return vec![self.black_level(bit_depth); count];
        };
        let Some(black) = &fidelity.black_levels else {
            let count = if mode == "linear" { 3 } else { 1 };
            return vec![self.black_level(bit_depth); count];
        };
        let white = self.effective_white(bit_depth, policy);
        let reference = fidelity
            .white_level
            .unwrap_or_else(|| Self::white_level(bit_depth));
        let mut values: Vec<u32> = black
            .iter()
            .map(|&(numerator, denominator)| {
                let scale = if denominator == 0 {
                    0.0
                } else {
                    f64::from(numerator) / f64::from(denominator)
                };
                let reference = f64::from(reference.max(1));
                let value = (scale * f64::from(white) / reference).round_ties_even();
                u32::try_from(value as u64).unwrap_or(u32::MAX).max(1)
            })
            .collect();
        if mode == "linear" && values.len() == 4 {
            values.truncate(3);
        }
        values
    }

    fn policy<'a>(&'a self, policy: &'a str) -> &'a str {
        if policy == "profile" {
            self.fidelity
                .as_ref()
                .map_or("capacity", |fidelity| fidelity.level_policy.as_str())
        } else {
            policy
        }
    }

    /// Build a profile from a registry `profile.json` value.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for malformed fields.
    pub fn from_json(
        data: &Value,
        assets: &dyn Fn(&str) -> Option<&'static [u8]>,
    ) -> Result<Self, ProfileError> {
        let data = strip_nulls(data.clone());
        let object = data
            .as_object()
            .ok_or_else(|| ProfileError::Invalid("profile must be a dict".to_owned()))?;
        let reference = Self::default();
        let get_str = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut profile = Self {
            make: string_or(object, "make", &reference.make),
            model: string_or(object, "model", &reference.model),
            unique_camera_model: string_or(
                object,
                "unique_camera_model",
                object
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(&reference.model),
            ),
            software: string_or(object, "software", &reference.software),
            dng_version: parse_version(object.get("dng_version"), reference.dng_version)?,
            dng_backward: parse_version(object.get("dng_backward"), reference.dng_backward)?,
            color_matrix1: parse_matrix(object.get("color_matrix1"), reference.color_matrix1)?,
            color_matrix2: parse_matrix(object.get("color_matrix2"), reference.color_matrix2)?,
            calib_illuminant1: u16_or(object, "calib_illuminant1", reference.calib_illuminant1)?,
            calib_illuminant2: u16_or(object, "calib_illuminant2", reference.calib_illuminant2)?,
            baseline_noise: pair_or(object.get("baseline_noise"), reference.baseline_noise)?,
            baseline_sharpness: pair_or(
                object.get("baseline_sharpness"),
                reference.baseline_sharpness,
            )?,
            linear_response_limit: pair_or(
                object.get("linear_response_limit"),
                reference.linear_response_limit,
            )?,
            shadow_scale: pair_or(object.get("shadow_scale"), reference.shadow_scale)?,
            black_level_ref: u32_or(object, "black_level_ref", reference.black_level_ref)?,
            opcode_list3: match object.get("opcode_list3_hex").and_then(Value::as_str) {
                Some(hex) => decode_hex(hex).ok_or_else(|| {
                    ProfileError::Invalid("opcode_list3_hex must be valid hex".to_owned())
                })?,
                None => reference.opcode_list3.clone(),
            },
            slug: get_str("slug"),
            display_name: get_str("display_name"),
            native_width: optional_usize(object.get("native_width"))?,
            native_height: optional_usize(object.get("native_height"))?,
            suggested_bit_depth: u8_or(object, "suggested_bit_depth", 16)?,
            cfa_pattern: object
                .get("cfa_pattern")
                .and_then(Value::as_str)
                .map(str::to_owned),
            crop_factor: object
                .get("crop_factor")
                .and_then(Value::as_f64)
                .unwrap_or(reference.crop_factor),
            lenses: parse_lenses(object.get("lenses"))?,
            iso_choices: parse_u32_list(object.get("iso_choices"), "iso_choices")?,
            exposure_choices: parse_rational_list(
                object.get("exposure_choices"),
                "exposure_choices",
            )?,
            fnumber_choices: parse_rational_list(object.get("fnumber_choices"), "fnumber_choices")?,
            metering_choices: parse_u16_list(object.get("metering_choices"), "metering_choices")?,
            exposure_program_choices: parse_u16_list(
                object.get("exposure_program_choices"),
                "exposure_program_choices",
            )?,
            serial_prefixes: parse_string_list(object.get("serial_prefixes"))
                .unwrap_or_else(|| reference.serial_prefixes.clone()),
            subprofiles: parse_subprofiles(object.get("subprofiles"))?,
            calibration_source: get_str("calibration_source"),
            fidelity: object
                .get("fidelity")
                .map(|value| FidelityConfig::from_json(value, assets))
                .transpose()?,
            gainmap: object
                .get("gainmap")
                .map(GainMapConfig::from_json)
                .transpose()?,
        };
        if let Some(adobe) = object.get("adobe") {
            let _ = assets;
            let _ = adobe;
        }
        if profile.slug.is_empty() {
            profile.slug = get_str("display_name");
        }
        Ok(profile)
    }
}

/// Fidelity level policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LevelPolicy {
    /// Use the profile's `raw.level_policy`.
    #[default]
    Profile,
    /// Keep full container range.
    Capacity,
    /// Honor the profile white level and plane ceiling.
    Strict,
}

impl LevelPolicy {
    /// Policy name understood by [`CameraProfile::effective_white`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Capacity => "capacity",
            Self::Strict => "strict",
        }
    }
}

/// Profile parse failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// Malformed or out-of-range field.
    #[error("invalid camera profile: {0}")]
    Invalid(String),
}

/// Highest writable plane count for a depth.
#[must_use]
pub const fn top_planes(bit_depth: u8) -> u8 {
    CameraProfile::top_planes(bit_depth)
}

/// Storage width: 8-bit fits `u8`, everything else uses `u16`.
#[must_use]
pub const fn container_bits(bit_depth: u8) -> u8 {
    CameraProfile::container_bits(bit_depth)
}

/// White level for a depth (`2**depth - 1`).
#[must_use]
pub const fn white_level(bit_depth: u8) -> u32 {
    CameraProfile::white_level(bit_depth)
}

/// Scaled black level using the GFX reference value.
#[must_use]
pub fn black_level(bit_depth: u8, reference: u32) -> u32 {
    let peak = white_level(bit_depth);
    let value = (u64::from(reference) * u64::from(peak) + 32_767) / 65_535;
    u32::try_from(value).unwrap_or(u32::MAX).max(1)
}
