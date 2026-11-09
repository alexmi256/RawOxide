//! Offline plausible capture locations.
//!
//! Mirrors Python `stegodng/geo/`: `zones.bin` carries one principal location
//! per canonical time zone, a 0.5 km land bitmap for a 25 km disk, and the
//! city elevation. Sampling rejects water and shrinks the radius, keeping
//! coastal cities on land without network access.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, FixedOffset, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use flate2::read::ZlibDecoder;
use thiserror::Error;

use crate::metadata::Rng64;

/// The land grid covers a disk of this radius around each zone centre.
pub const MAX_RADIUS_KM: f64 = 25.0;
/// Land bitmap resolution inside that disk.
pub const CELL_KM: f64 = 0.5;
/// Land grid side length.
pub const GRID: usize = 100;
/// Bytes in one zone land mask.
pub const MASK_BYTES: usize = (GRID * GRID).div_ceil(8);
const MAGIC: &[u8; 6] = b"STGEO1";
const KM_PER_DEG: f64 = 111.32;
const SHRINK: f64 = 0.55;
const TRIES: usize = 14;

/// One principal tz location and the land around it.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    /// IANA zone name.
    pub name: String,
    /// Centre latitude in degrees.
    pub lat: f64,
    /// Centre longitude in degrees.
    pub lon: f64,
    /// City elevation in metres.
    pub elevation_m: i16,
    /// Land bitmap.
    pub mask: Vec<u8>,
}

impl Zone {
    /// Whether grid cell `(i, j)` is land.
    #[must_use]
    pub fn cell_is_land(&self, i: usize, j: usize) -> bool {
        if i >= GRID || j >= GRID {
            return false;
        }
        let bit = j * GRID + i;
        self.mask
            .get(bit >> 3)
            .is_some_and(|byte| byte & (1 << (bit & 7)) != 0)
    }

    /// Offset of this zone at a local wall time.
    pub fn offset_at(&self, local_time: NaiveDateTime) -> Result<FixedOffset, GeoError> {
        offset_at(&self.name, local_time)
    }

    /// `±HH:MM` offset at a local wall time.
    #[must_use]
    pub fn offset_string(&self, local_time: NaiveDateTime) -> Option<String> {
        self.offset_at(local_time).ok().map(format_offset)
    }
}

/// A sampled capture point, altitude already drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    /// IANA zone name.
    pub zone: String,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// Altitude in metres.
    pub altitude_m: f64,
}

/// Location sampling failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GeoError {
    /// Radius outside the offline grid.
    #[error(
        "gps radius must be between 0 and {MAX_RADIUS_KM} km (got {value}); \
         the offline land grid covers 25 km"
    )]
    InvalidRadius {
        /// Requested radius.
        value: String,
    },

    /// Embedded data is missing or corrupt.
    #[error("invalid location data: {0}")]
    InvalidData(String),

    /// Forced zone is not in the table.
    #[error("unknown location zone {0:?}")]
    UnknownZone(String),

    /// Time-zone data unavailable for a zone name.
    #[error("unknown time zone {0:?}")]
    UnknownTimeZone(String),

    /// Offset filtering needs a local time.
    #[error("local_time is required when filtering by offset")]
    MissingLocalTime,
}

/// Validate a radius against the offline grid.
///
/// # Errors
///
/// Returns [`GeoError::InvalidRadius`] for non-finite or out-of-range values.
pub fn validate_radius(radius_km: f64) -> Result<f64, GeoError> {
    if !radius_km.is_finite() || radius_km <= 0.0 || radius_km > MAX_RADIUS_KM {
        return Err(GeoError::InvalidRadius {
            value: radius_km.to_string(),
        });
    }
    Ok(radius_km)
}

fn parse_zone(blob: &[u8]) -> Result<Vec<Zone>, GeoError> {
    if !blob.starts_with(MAGIC) {
        return Err(GeoError::InvalidData("bad header".to_owned()));
    }
    let mut position = MAGIC.len();
    let count = read_u16(blob, &mut position)? as usize;
    let mut zones = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(read_u8(blob, &mut position)?);
        let name = std::str::from_utf8(read_slice(blob, &mut position, length)?)
            .map_err(|_| GeoError::InvalidData("zone name is not UTF-8".to_owned()))?
            .to_owned();
        let lat = read_i32(blob, &mut position)?;
        let lon = read_i32(blob, &mut position)?;
        let mask_len = usize::from(read_u16(blob, &mut position)?);
        let elevation = read_i16(blob, &mut position)?;
        if mask_len != MASK_BYTES {
            return Err(GeoError::InvalidData(format!(
                "bad land mask for {name}: {mask_len} bytes"
            )));
        }
        let mask = read_slice(blob, &mut position, mask_len)?.to_vec();
        zones.push(Zone {
            name,
            lat: f64::from(lat) / 1_000_000.0,
            lon: f64::from(lon) / 1_000_000.0,
            elevation_m: elevation,
            mask,
        });
    }
    if zones.is_empty() {
        return Err(GeoError::InvalidData("no locations".to_owned()));
    }
    if position != blob.len() {
        return Err(GeoError::InvalidData(format!(
            "truncated data: {position} of {} bytes",
            blob.len()
        )));
    }
    Ok(zones)
}

fn read_u8(blob: &[u8], position: &mut usize) -> Result<u8, GeoError> {
    let value = *blob
        .get(*position)
        .ok_or_else(|| GeoError::InvalidData("truncated".to_owned()))?;
    *position += 1;
    Ok(value)
}

fn read_u16(blob: &[u8], position: &mut usize) -> Result<u16, GeoError> {
    let bytes = read_slice(blob, position, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_i16(blob: &[u8], position: &mut usize) -> Result<i16, GeoError> {
    let bytes = read_slice(blob, position, 2)?;
    Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_i32(blob: &[u8], position: &mut usize) -> Result<i32, GeoError> {
    let bytes = read_slice(blob, position, 4)?;
    Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_slice<'a>(
    blob: &'a [u8],
    position: &mut usize,
    length: usize,
) -> Result<&'a [u8], GeoError> {
    let end = position
        .checked_add(length)
        .ok_or_else(|| GeoError::InvalidData("truncated".to_owned()))?;
    let slice = blob
        .get(*position..end)
        .ok_or_else(|| GeoError::InvalidData("truncated".to_owned()))?;
    *position = end;
    Ok(slice)
}

/// The vendored principal locations, loaded once.
///
/// # Errors
///
/// Returns [`GeoError::InvalidData`] when the embedded blob is corrupt.
pub fn zones() -> Result<&'static [Zone], GeoError> {
    static ZONES: OnceLock<Result<Vec<Zone>, GeoError>> = OnceLock::new();
    ZONES
        .get_or_init(|| {
            let compressed = include_bytes!("../../../assets/geo/zones.bin");
            let mut decoder = ZlibDecoder::new(&compressed[..]);
            let mut blob = Vec::new();
            decoder
                .read_to_end(&mut blob)
                .map_err(|error| GeoError::InvalidData(error.to_string()))?;
            parse_zone(&blob)
        })
        .as_deref()
        .map_err(Clone::clone)
}

/// UTC offset of `zone_name` at a local wall time.
///
/// Ambiguous times use the earliest offset; nonexistent spring-forward times
/// use the pre-transition offset, matching Python `ZoneInfo` fold=0.
///
/// # Errors
///
/// Returns [`GeoError::UnknownTimeZone`] for unknown names.
pub fn offset_at(zone_name: &str, local_time: NaiveDateTime) -> Result<FixedOffset, GeoError> {
    let zone: Tz = zone_name
        .parse()
        .map_err(|_| GeoError::UnknownTimeZone(zone_name.to_owned()))?;
    if let Some(offset) = zone
        .offset_from_local_datetime(&local_time)
        .earliest()
        .map(|offset| offset.fix())
    {
        return Ok(offset);
    }
    // Spring-forward gaps have no local time. Python `ZoneInfo` fold=0
    // returns the pre-transition offset, so search backwards for the
    // nearest valid local time and use its offset.
    for minutes in [1_i64, 15, 60, 180, 360, 720, 1_440, 2_880] {
        if let Some(candidate) = local_time.checked_sub_signed(chrono::Duration::minutes(minutes))
            && let Some(offset) = zone
                .offset_from_local_datetime(&candidate)
                .earliest()
                .map(|offset| offset.fix())
        {
            return Ok(offset);
        }
    }
    Ok(zone.offset_from_utc_datetime(&local_time).fix())
}

/// `±HH:MM` for a UTC offset.
#[must_use]
pub fn format_offset(offset: FixedOffset) -> String {
    format_offset_seconds(offset.local_minus_utc())
}

fn format_offset_seconds(seconds: i32) -> String {
    let sign = if seconds >= 0 { '+' } else { '-' };
    let minutes = seconds.unsigned_abs() / 60;
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Naive UTC wall time for an aware instant.
#[must_use]
pub fn utc_time(instant: DateTime<FixedOffset>) -> NaiveDateTime {
    instant.with_timezone(&Utc).naive_utc()
}

fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0) % 360.0 - 180.0
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn cell_of(zone: &Zone, lat: f64, lon: f64) -> (usize, usize) {
    let y = (lat - zone.lat) * KM_PER_DEG;
    let coslat = f64::max(0.05, zone.lat.to_radians().cos());
    let x = wrap_lon(lon - zone.lon) * KM_PER_DEG * coslat;
    let i = ((x + MAX_RADIUS_KM) / CELL_KM).clamp(0.0, (GRID - 1) as f64) as usize;
    let j = ((y + MAX_RADIUS_KM) / CELL_KM).clamp(0.0, (GRID - 1) as f64) as usize;
    (i, j)
}

type LandCache = Mutex<BTreeMap<String, Option<(usize, usize)>>>;

#[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
fn nearest_land_cell(zone: &Zone) -> Option<(usize, usize)> {
    static CACHE: OnceLock<LandCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(cached) = cache.get(&zone.name)
    {
        return *cached;
    }
    let mut best: Option<(f64, usize, usize)> = None;
    for j in 0..GRID {
        let y = (j as f64 + 0.5) * CELL_KM - MAX_RADIUS_KM;
        for i in 0..GRID {
            if !zone.cell_is_land(i, j) {
                continue;
            }
            let x = (i as f64 + 0.5) * CELL_KM - MAX_RADIUS_KM;
            let distance = x.hypot(y);
            if best.is_none_or(|(current, _, _)| distance < current) {
                best = Some((distance, i, j));
            }
        }
    }
    let result = best.map(|(_, i, j)| (i, j));
    if let Ok(mut cache) = cache.lock() {
        cache.insert(zone.name.clone(), result);
    }
    result
}

fn candidates<'a>(
    table: &'a [Zone],
    local_time: Option<NaiveDateTime>,
    offset: Option<&str>,
) -> Result<Vec<&'a Zone>, GeoError> {
    let Some(offset) = offset else {
        return Ok(table.iter().collect());
    };
    let Some(local_time) = local_time else {
        return Err(GeoError::MissingLocalTime);
    };
    let matching: Vec<&Zone> = table
        .iter()
        .filter(|zone| zone.offset_string(local_time).as_deref() == Some(offset))
        .collect();
    if matching.is_empty() {
        Ok(table.iter().collect())
    } else {
        Ok(matching)
    }
}

/// Pick a random land point within `radius_km` of a principal location.
///
/// # Errors
///
/// Returns [`GeoError`] for invalid radii, missing data, an unknown forced
/// zone, or unavailable time-zone data.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#[allow(clippy::many_single_char_names, clippy::suboptimal_flops)]
pub fn sample_location(
    rng: &mut Rng64,
    radius_km: f64,
    local_time: Option<NaiveDateTime>,
    offset: Option<&str>,
    forced_zone: Option<&str>,
) -> Result<Location, GeoError> {
    let radius = validate_radius(radius_km)?;
    let table = zones()?;
    let candidates = if let Some(name) = forced_zone {
        let matching: Vec<&Zone> = table.iter().filter(|zone| zone.name == name).collect();
        if matching.is_empty() {
            return Err(GeoError::UnknownZone(name.to_owned()));
        }
        matching
    } else {
        candidates(table, local_time, offset)?
    };
    let chosen = *rng.choice(&candidates);
    let span = MAX_RADIUS_KM;
    let mut r = radius;
    let mut x = 0.0;
    let mut y = 0.0;
    let mut landed = false;
    for _ in 0..TRIES {
        let distance = r * rng.uniform(0.0, 1.0).sqrt();
        let theta = rng.uniform(0.0, 2.0 * std::f64::consts::PI);
        x = distance * theta.cos();
        y = distance * theta.sin();
        let i = ((x + span) / CELL_KM).clamp(0.0, (GRID - 1) as f64) as usize;
        let j = ((y + span) / CELL_KM).clamp(0.0, (GRID - 1) as f64) as usize;
        if chosen.cell_is_land(i, j) {
            landed = true;
            break;
        }
        r *= SHRINK;
    }
    if !landed {
        if let Some((i, j)) = nearest_land_cell(chosen) {
            x = (i as f64 + 0.5) * CELL_KM - span + rng.uniform(-0.2, 0.2);
            y = (j as f64 + 0.5) * CELL_KM - span + rng.uniform(-0.2, 0.2);
        } else {
            x = 0.0;
            y = 0.0;
        }
    }
    let coslat = f64::max(0.05, chosen.lat.to_radians().cos());
    let lat = chosen.lat + y / KM_PER_DEG;
    let lon = wrap_lon(chosen.lon + x / (KM_PER_DEG * coslat));
    let altitude = f64::from(chosen.elevation_m) + rng.uniform(-20.0, 20.0);
    Ok(Location {
        zone: chosen.name.clone(),
        lat,
        lon,
        altitude_m: altitude,
    })
}

/// Convert degrees to EXIF degrees/minutes/seconds (seconds over 100).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn dms(value: f64) -> (u32, u32, (u32, u32)) {
    let hundredths = (value.abs() * 360_000.0).round() as u64;
    let degrees = hundredths / 360_000;
    let rest = hundredths % 360_000;
    let minutes = rest / 6_000;
    let seconds = rest % 6_000;
    (degrees as u32, minutes as u32, (seconds as u32, 100))
}

/// Point-in-land check for tests/debugging.
#[must_use]
pub fn point_is_land(zone: &Zone, lat: f64, lon: f64) -> bool {
    let (i, j) = cell_of(zone, lat, lon);
    zone.cell_is_land(i, j)
}

/// Number of principal locations.
///
/// # Errors
///
/// Returns [`GeoError::InvalidData`] when the embedded blob is corrupt.
pub fn count() -> Result<usize, GeoError> {
    Ok(zones()?.len())
}

/// Validate a zone name for tests and fixtures.
///
/// # Errors
///
/// Returns [`GeoError::UnknownTimeZone`] when chrono-tz cannot resolve it.
pub fn validate_zone(name: &str) -> Result<Tz, GeoError> {
    name.parse()
        .map_err(|_| GeoError::UnknownTimeZone(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_parses() {
        let table = zones().unwrap();
        assert!(table.len() > 250);
        assert!(table.iter().any(|zone| zone.name == "Asia/Tokyo"));
        assert!(table.iter().all(|zone| zone.mask.len() == MASK_BYTES));
    }

    #[test]
    fn forced_zone_sample_stays_near_center_and_on_land() {
        let table = zones().unwrap();
        let zone = table.iter().find(|zone| zone.name == "Asia/Tokyo").unwrap();
        let mut rng = Rng64::new(Some(3));
        for _ in 0..10 {
            let location =
                sample_location(&mut rng, MAX_RADIUS_KM, None, None, Some("Asia/Tokyo")).unwrap();
            assert!((location.lat - zone.lat).abs() < 0.3);
            assert!((location.lon - zone.lon).abs() < 0.3);
            assert!(point_is_land(zone, location.lat, location.lon));
        }
    }

    #[test]
    fn offset_filter_matches_requested_offset() {
        let time =
            NaiveDateTime::parse_from_str("2021:06:05 14:30:09", "%Y:%m:%d %H:%M:%S").unwrap();
        let mut rng = Rng64::new(Some(5));
        let location =
            sample_location(&mut rng, MAX_RADIUS_KM, Some(time), Some("+09:00"), None).unwrap();
        assert_eq!(
            offset_at(&location.zone, time).unwrap().local_minus_utc(),
            9 * 3600
        );
    }

    #[test]
    fn invalid_radius_and_unknown_zone_fail() {
        assert!(matches!(
            validate_radius(0.0),
            Err(GeoError::InvalidRadius { .. })
        ));
        assert!(matches!(
            validate_radius(25.1),
            Err(GeoError::InvalidRadius { .. })
        ));
        let mut rng = Rng64::new(Some(1));
        assert!(matches!(
            sample_location(&mut rng, MAX_RADIUS_KM, None, None, Some("Not/AZone")),
            Err(GeoError::UnknownZone(_))
        ));
    }

    #[test]
    fn offset_filter_needs_local_time() {
        let mut rng = Rng64::new(Some(1));
        assert!(matches!(
            sample_location(&mut rng, MAX_RADIUS_KM, None, Some("+09:00"), None),
            Err(GeoError::MissingLocalTime)
        ));
    }

    #[test]
    fn unmatched_offset_falls_back_to_the_whole_table() {
        let time =
            NaiveDateTime::parse_from_str("2021:06:05 14:30:09", "%Y:%m:%d %H:%M:%S").unwrap();
        let mut rng = Rng64::new(Some(5));
        let location =
            sample_location(&mut rng, MAX_RADIUS_KM, Some(time), Some("+12:34"), None).unwrap();
        assert_ne!(location.zone, "");
    }

    #[test]
    fn dst_gap_uses_pre_transition_offset() {
        let sydney =
            NaiveDateTime::parse_from_str("2024:10:06 02:30:00", "%Y:%m:%d %H:%M:%S").unwrap();
        assert_eq!(
            offset_at("Australia/Sydney", sydney)
                .unwrap()
                .local_minus_utc(),
            10 * 3600
        );
        let new_york =
            NaiveDateTime::parse_from_str("2024:03:10 02:30:00", "%Y:%m:%d %H:%M:%S").unwrap();
        assert_eq!(
            offset_at("America/New_York", new_york)
                .unwrap()
                .local_minus_utc(),
            -5 * 3600
        );
        // Samoa skipped 2011-12-30 entirely; Python still folds to the
        // pre-transition -10:00 offset.
        let apia =
            NaiveDateTime::parse_from_str("2011:12:30 12:00:00", "%Y:%m:%d %H:%M:%S").unwrap();
        assert_eq!(
            offset_at("Pacific/Apia", apia).unwrap().local_minus_utc(),
            -10 * 3600
        );
    }

    #[test]
    fn nearest_land_cell_finds_land_for_coastal_and_inland_zones() {
        for name in [
            "Asia/Tokyo",
            "Europe/London",
            "America/New_York",
            "Asia/Ulaanbaatar",
        ] {
            let zone = zones()
                .unwrap()
                .iter()
                .find(|zone| zone.name == name)
                .unwrap();
            let (i, j) = nearest_land_cell(zone).expect("land cell");
            assert!(zone.cell_is_land(i, j), "{name} fallback is not land");
        }
    }

    #[test]
    fn corrupt_zone_data_is_rejected() {
        assert!(parse_zone(b"nope").is_err());
        assert!(parse_zone(b"STGEO1\x01\x00\x05").is_err());
    }

    #[test]
    fn dms_carries_cleanly() {
        assert_eq!(dms(35.681_236), (35, 40, (5245, 100)));
        assert_eq!(dms(-0.5), (0, 30, (0, 100)));
    }
}
