//! Per-file metadata randomization.
//!
//! Mirrors Python `stegodng/metadata.py`: static calibration stays fixed while
//! timestamps, exposure, serials, lens choice and (when the profile opts in)
//! GPS are re-rolled per file. The RNG is a local `SplitMix64` stream, so output
//! is reproducible for a fixed seed without depending on Python's Mersenne
//! Twister draw order.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, Offset, TimeZone};

use crate::geo::Location;
use crate::profile::{CameraProfile, FidelityConfig, Lens};

/// Reference GFX lens pool.
pub const LENSES: [(&str, f64, f64, f64); 8] = [
    ("GF63mmF2.8 R WR", 63.0, 63.0, 2.8),
    ("GF120mmF4 R LM OIS WR Macro", 120.0, 120.0, 4.0),
    ("GF55mmF1.7 R WR", 55.0, 55.0, 1.7),
    ("GF110mmF2 R LM WR", 110.0, 110.0, 2.0),
    ("GF32-64mmF4 R LM WR", 32.0, 64.0, 4.0),
    ("GF100-200mmF5.6 R LM OIS WR", 100.0, 200.0, 5.6),
    ("GF23mmF4 R LM WR", 23.0, 23.0, 4.0),
    ("GF250mmF4 R LM OIS WR", 250.0, 250.0, 4.0),
];

/// Reference ISO pool.
pub const ISO_CHOICES: [u32; 17] = [
    50, 64, 100, 125, 160, 200, 250, 320, 400, 500, 640, 800, 1000, 1250, 1600, 3200, 6400,
];

/// Reference exposure-time pool.
pub const EXPOSURE_CHOICES: [(u32, u32); 23] = [
    (10, 8000),
    (10, 4000),
    (10, 2000),
    (10, 1000),
    (10, 500),
    (10, 250),
    (10, 125),
    (10, 60),
    (10, 30),
    (10, 21),
    (10, 17),
    (10, 16),
    (10, 8),
    (10, 5),
    (10, 4),
    (10, 3),
    (10, 2),
    (10, 1),
    (15, 10),
    (20, 10),
    (30, 10),
    (50, 10),
    (60, 10),
];

/// Reference f-number pool (x100).
pub const FNUMBER_CHOICES: [u32; 13] = [
    170, 200, 250, 280, 320, 400, 560, 710, 800, 1100, 1600, 2200, 3200,
];

/// Reference metering pool.
pub const METERING_CHOICES: [u16; 3] = [2, 3, 5];

/// Reference exposure-program pool.
pub const EXPPROG_CHOICES: [u16; 4] = [1, 2, 3, 4];

/// GPS metadata; `zone` is debug-only and never written to EXIF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpsMetadata {
    /// Latitude reference (`N`/`S`).
    pub lat_ref: char,
    /// Latitude degrees/minutes/seconds (seconds over 100).
    pub lat: (u32, u32, (u32, u32)),
    /// Longitude reference (`E`/`W`).
    pub lon_ref: char,
    /// Longitude degrees/minutes/seconds (seconds over 100).
    pub lon: (u32, u32, (u32, u32)),
    /// Altitude reference (0 above sea level, 1 below).
    pub altitude_ref: u16,
    /// Absolute altitude in millimetres.
    pub altitude_mm: u32,
    /// UTC hour/minute/second.
    pub time: (u32, u32, u32),
    /// GPS dilution of precision.
    pub dop: u32,
    /// UTC date (`YYYY:MM:DD`).
    pub date: String,
    /// IANA zone for debug sidecars.
    pub zone: String,
}

/// Pinned capture time and its EXIF `OffsetTime` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageTime {
    /// Local wall-clock capture time.
    pub datetime: NaiveDateTime,
    /// EXIF offset (`±HH:MM`).
    pub offset: String,
}

impl ImageTime {
    /// Build from a local wall-clock time, using the system offset.
    #[must_use]
    pub fn local(datetime: NaiveDateTime) -> Self {
        let offset = local_offset(datetime, |candidate| {
            Local
                .from_local_datetime(&candidate)
                .earliest()
                .map(|value| value.offset().fix())
        });
        Self { datetime, offset }
    }

    /// Build from an aware timestamp, converting to local time like Python.
    #[must_use]
    pub fn from_aware(value: &DateTime<chrono::FixedOffset>) -> Self {
        let local = value.with_timezone(&Local);
        Self {
            datetime: local.naive_local(),
            offset: format_offset(local.offset().local_minus_utc()),
        }
    }
}

fn format_offset(seconds: i32) -> String {
    let sign = if seconds >= 0 { '+' } else { '-' };
    let minutes = seconds.unsigned_abs() / 60;
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Randomized per-file metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    /// Capture timestamp (`YYYY:MM:DD HH:MM:SS`).
    pub datetime: String,
    /// Software string.
    pub software: String,
    /// EXIF orientation.
    pub orientation: u16,
    /// Subject distance rational.
    pub subject_distance: (u32, u32),
    /// Subject distance range.
    pub subject_distance_range: u16,
    /// GPS block.
    pub gps: Option<GpsMetadata>,
    /// Noise profile.
    pub noise_profile: Option<[f64; 6]>,
    /// Exposure time.
    pub exposure_time: (u32, u32),
    /// F-number (x100).
    pub fnumber: (u32, u32),
    /// Exposure program.
    pub exposure_program: u16,
    /// ISO speed.
    pub iso: u32,
    /// Metering mode.
    pub metering: u16,
    /// Focal length in mm.
    pub focal: f64,
    /// Focal length rational (x100).
    pub focal_rational: (u32, u32),
    /// 35 mm-equivalent focal length.
    pub focal_35mm: u32,
    /// Maximum aperture (x100).
    pub max_aperture: (u32, u32),
    /// Lens model.
    pub lens: String,
    /// Lens make.
    pub lens_make: String,
    /// Lens specification quad.
    pub lens_spec: [(u32, u32); 4],
    /// Camera serial.
    pub camera_serial: String,
    /// Body serial.
    pub body_serial: String,
    /// Lens serial.
    pub lens_serial: String,
    /// Image number.
    pub image_number: u32,
    /// Sequence number.
    pub sequence: u32,
    /// `AsShotNeutral` (x10000).
    pub as_shot_neutral: [(i32, u32); 3],
    /// `BaselineExposure` (`EV` x10000).
    pub baseline_exposure: (i32, u32),
    /// Brightness (x100).
    pub brightness: (u32, u32),
    /// Exposure bias (`EV` x100).
    pub exposure_bias: (i32, u32),
    /// Flash tag value.
    pub flash: u16,
    /// Sub-second fraction.
    pub subsec: String,
    /// Offset time string.
    pub offset: String,
}

/// `SplitMix64` with rejection-free bounded draws.
#[derive(Debug, Clone)]
pub struct Rng64 {
    state: u64,
}

impl Rng64 {
    /// Create a stream from an explicit seed or fresh entropy.
    #[must_use]
    pub fn new(seed: Option<u64>) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seed = seed.unwrap_or_else(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos() as u64);
            nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(17)
        });
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Next raw value.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// Uniform integer in `0..bound` (bound > 0).
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        ((u128::from(self.next_u64()) * u128::from(bound)) >> 64) as u64
    }

    /// Inclusive integer range.
    pub fn range(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        let span = (high - low) as u64 + 1;
        low + self.below(span) as i64
    }

    /// Uniform float in `[low, high)`.
    #[allow(clippy::cast_precision_loss)]
    pub fn uniform(&mut self, low: f64, high: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64;
        (high - low).mul_add(unit, low)
    }

    /// Pick one item from a non-empty slice.
    pub fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Seeded metadata randomizer for one profile.
#[derive(Debug, Clone)]
pub struct MetadataRandomizer {
    rng: Rng64,
    location_rng: Rng64,
    profile: Option<CameraProfile>,
    date_bounds: Option<(NaiveDateTime, NaiveDateTime)>,
    gps_radius_km: f64,
}

impl MetadataRandomizer {
    /// Create a randomizer for `profile` (defaults to the GFX pools).
    #[must_use]
    pub fn new(seed: Option<u64>, profile: Option<&CameraProfile>) -> Self {
        let date_bounds = profile
            .and_then(|profile| profile.fidelity.as_ref())
            .and_then(FidelityConfig::date_bounds);
        Self {
            rng: Rng64::new(seed),
            location_rng: Rng64::new(seed),
            profile: profile.cloned(),
            date_bounds,
            gps_radius_km: crate::geo::MAX_RADIUS_KM,
        }
    }

    /// Override the GPS sampling radius.
    ///
    /// # Errors
    ///
    /// Returns [`crate::geo::GeoError::InvalidRadius`] outside `(0, 25] km`.
    pub fn with_gps_radius(mut self, radius_km: f64) -> Result<Self, crate::geo::GeoError> {
        self.gps_radius_km = crate::geo::validate_radius(radius_km)?;
        Ok(self)
    }

    /// Fresh metadata with identifiable fields randomized.
    #[must_use]
    pub fn randomize(&mut self, image_time: Option<&ImageTime>) -> Metadata {
        self.randomize_with_location(image_time, None)
    }

    /// Fresh metadata, reusing a GPS location sampled for a split set.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn randomize_with_location(
        &mut self,
        image_time: Option<&ImageTime>,
        shared_location: Option<&Location>,
    ) -> Metadata {
        let profile = self.profile.clone();
        let profile = profile.as_ref();
        let fidelity = profile.and_then(|profile| profile.fidelity.as_ref());

        let lens = self.pick_lens(profile);
        let lens_make = lens
            .make
            .clone()
            .or_else(|| profile.map(|profile| profile.make.clone()))
            .unwrap_or_else(|| "FUJIFILM".to_owned());
        let focal = (self.rng.uniform(lens.min_mm, lens.max_mm) * 10.0).round() / 10.0;
        let iso = self.pick_iso(profile);
        let exposure = self.pick_exposure(profile);
        let fnumber = self.pick_fnumber(profile);

        let base = if let Some(image_time) = image_time {
            image_time.datetime
        } else if let Some((start, end)) = self.date_bounds {
            let span = (end - start).num_seconds().max(1);
            start + Duration::seconds(self.rng.range(0, span))
        } else {
            NaiveDate::from_ymd_opt(2019, 1, 1)
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .unwrap_or_default()
                + Duration::seconds(self.rng.range(0, 5 * 365 * 24 * 3600))
        };
        let datetime = base.format("%Y:%m:%d %H:%M:%S").to_string();

        let software = if let Some(fidelity) = fidelity {
            if fidelity.software_variants.is_empty() {
                profile.map_or_else(String::new, |profile| profile.software.clone())
            } else {
                self.rng.choice(&fidelity.software_variants).clone()
            }
        } else {
            profile.map_or_else(String::new, |profile| profile.software.clone())
        };
        let orientation =
            if let Some(fidelity) = fidelity.filter(|fidelity| !fidelity.orientations.is_empty()) {
                *self.rng.choice(&fidelity.orientations)
            } else {
                1
            };
        let (subject_distance, subject_distance_range) = if let Some(fidelity) =
            fidelity.filter(|fidelity| !fidelity.subject_distance.is_empty())
        {
            let distance = *self.rng.choice(&fidelity.subject_distance);
            let range = if matches!(distance.0, 0 | u32::MAX) {
                3
            } else {
                let metres = if distance.1 == 0 {
                    0.0
                } else {
                    f64::from(distance.0) / f64::from(distance.1)
                };
                if metres <= 1.0 {
                    1
                } else if metres <= 2.0 {
                    2
                } else {
                    3
                }
            };
            (distance, range)
        } else {
            ((0, 0), 0)
        };

        let noise_profile = match (
            fidelity.and_then(|f| f.noise_s),
            fidelity.and_then(|f| f.noise_o),
        ) {
            (Some(s), Some(o)) => {
                let s = self.rng.uniform(s.0.ln(), s.1.ln()).exp();
                let o = self.rng.uniform(o.0.ln(), o.1.ln()).exp();
                Some([s, o, s, o, s, o])
            }
            _ => None,
        };

        let serial_prefixes = profile.map_or_else(
            || {
                vec![
                    "92A".to_owned(),
                    "94A".to_owned(),
                    "93A".to_owned(),
                    "95A".to_owned(),
                ]
            },
            |profile| profile.serial_prefixes.clone(),
        );
        let serial_prefix = self.rng.choice(&serial_prefixes).clone();
        let serial = self.rand_serial(serial_prefix);
        let legacy = profile.is_none_or(|profile| profile.slug == "gfx_100");
        let body_serial = if legacy {
            let options = ["33000087", "94000525"];
            if self.rng.below(3) == 0 {
                serial.clone()
            } else {
                self.rng.choice(&options).to_string()
            }
        } else {
            let prefix = self.rng.choice(&serial_prefixes).clone();
            self.rand_serial(prefix)
        };
        let lens_serial = if legacy {
            let prefixes = ["75A", "86A", "35A", "19A"];
            let prefix = self.rng.choice(&prefixes).to_string();
            self.rand_serial(prefix)
        } else {
            let prefix = self.rng.choice(&serial_prefixes).clone();
            self.rand_serial(prefix)
        };

        let crop = profile.map_or(0.79, |profile| profile.crop_factor);
        let focal_35mm = (focal * crop).round() as u32;
        let (asn_low, asn_high) = fidelity
            .and_then(|fidelity| fidelity.asn_r)
            .unwrap_or((5000, 7600));
        let (asn_blue_low, asn_blue_high) = fidelity
            .and_then(|fidelity| fidelity.asn_b)
            .unwrap_or((4000, 5600));
        let (baseline_low, baseline_high) = fidelity
            .and_then(|fidelity| fidelity.baseline_exposure)
            .unwrap_or((-60, 60));
        let exposure_bias = if let Some(fidelity) =
            fidelity.filter(|fidelity| !fidelity.exposure_bias.is_empty())
        {
            *self.rng.choice(&fidelity.exposure_bias)
        } else {
            *self.rng.choice(&[0, 0, 0, -33, 33, -67, 67])
        };
        let flash = fidelity.and_then(|fidelity| fidelity.flash).unwrap_or(0);
        let digits = fidelity.map_or(2, |fidelity| fidelity.subsec_digits);
        let subsec_max = 10_u64.pow(u32::from(digits));
        let subsec = format!(
            "{:0width$}",
            self.rng.below(subsec_max),
            width = usize::from(digits)
        );
        let mut gps = None;
        let mut gps_offset = None;
        if fidelity.is_some_and(|fidelity| fidelity.emit_gps) {
            let filter = image_time.map(|image_time| image_time.offset.as_str());
            let sampled = if shared_location.is_none() {
                crate::geo::sample_location(
                    &mut self.location_rng,
                    self.gps_radius_km,
                    Some(base),
                    filter,
                    None,
                )
                .ok()
            } else {
                None
            };
            if let Some(location) = shared_location.cloned().or(sampled) {
                let resolved = image_time
                    .and_then(|image_time| {
                        parse_offset_seconds(&image_time.offset)
                            .map(|seconds| (seconds, image_time.offset.clone()))
                    })
                    .or_else(|| {
                        crate::geo::offset_at(&location.zone, base)
                            .ok()
                            .map(|offset| {
                                (offset.local_minus_utc(), crate::geo::format_offset(offset))
                            })
                    })
                    .unwrap_or_else(|| (0, "+00:00".to_owned()));
                let utc = base - Duration::seconds(i64::from(resolved.0));
                let (lat_d, lat_m, lat_s) = crate::geo::dms(location.lat);
                let (lon_d, lon_m, lon_s) = crate::geo::dms(location.lon);
                gps = Some(GpsMetadata {
                    lat_ref: if location.lat >= 0.0 { 'N' } else { 'S' },
                    lat: (lat_d, lat_m, lat_s),
                    lon_ref: if location.lon >= 0.0 { 'E' } else { 'W' },
                    lon: (lon_d, lon_m, lon_s),
                    altitude_ref: u16::from(location.altitude_m < 0.0),
                    altitude_mm: (location.altitude_m.abs() * 1000.0).round() as u32,
                    time: (
                        utc.format("%H").to_string().parse().unwrap_or(0),
                        utc.format("%M").to_string().parse().unwrap_or(0),
                        utc.format("%S").to_string().parse().unwrap_or(0),
                    ),
                    dop: self.rng.range(5_000, 20_000) as u32,
                    date: utc.format("%Y:%m:%d").to_string(),
                    zone: location.zone,
                });
                gps_offset = Some(resolved.1);
            }
        }
        let offset = match image_time {
            Some(image_time) => image_time.offset.clone(),
            None => gps_offset.unwrap_or_else(|| {
                let options = ["+08:00", "+09:00", "+01:00", "+02:00", "-05:00"];
                self.rng.choice(&options).to_string()
            }),
        };

        Metadata {
            datetime,
            software,
            orientation,
            subject_distance,
            subject_distance_range,
            gps,
            noise_profile,
            exposure_time: exposure,
            fnumber,
            exposure_program: if let Some(profile) =
                profile.filter(|profile| !profile.exposure_program_choices.is_empty())
            {
                *self.rng.choice(&profile.exposure_program_choices)
            } else {
                *self.rng.choice(&EXPPROG_CHOICES)
            },
            iso,
            metering: if let Some(profile) =
                profile.filter(|profile| !profile.metering_choices.is_empty())
            {
                *self.rng.choice(&profile.metering_choices)
            } else {
                *self.rng.choice(&METERING_CHOICES)
            },
            focal,
            focal_rational: ((focal * 100.0).round() as u32, 100),
            focal_35mm,
            max_aperture: ((lens.max_aperture * 100.0).round() as u32, 100),
            lens: lens.model,
            lens_make,
            lens_spec: [
                ((lens.min_mm * 100.0).round() as u32, 100),
                ((lens.max_mm * 100.0).round() as u32, 100),
                ((lens.max_aperture * 100.0).round() as u32, 100),
                ((lens.max_aperture * 100.0).round() as u32, 100),
            ],
            camera_serial: serial,
            body_serial,
            lens_serial,
            image_number: self.rng.range(1000, 99_999) as u32,
            sequence: self.rng.below(10) as u32,
            as_shot_neutral: [
                (
                    self.rng.range(i64::from(asn_low), i64::from(asn_high)) as i32,
                    10_000,
                ),
                (10_000, 10_000),
                (
                    self.rng
                        .range(i64::from(asn_blue_low), i64::from(asn_blue_high))
                        as i32,
                    10_000,
                ),
            ],
            baseline_exposure: (
                self.rng
                    .range(i64::from(baseline_low), i64::from(baseline_high))
                    as i32,
                10_000,
            ),
            brightness: (self.rng.range(100, 600) as u32, 100),
            exposure_bias: (exposure_bias, 100),
            flash,
            subsec,
            offset,
        }
    }

    fn pick_lens(&mut self, profile: Option<&CameraProfile>) -> Lens {
        if let Some(profile) = profile.filter(|profile| !profile.lenses.is_empty()) {
            self.rng.choice(&profile.lenses).clone()
        } else {
            let (model, min_mm, max_mm, max_aperture) = *self.rng.choice(&LENSES);
            Lens {
                model: model.to_owned(),
                min_mm,
                max_mm,
                max_aperture,
                make: None,
            }
        }
    }

    fn pick_iso(&mut self, profile: Option<&CameraProfile>) -> u32 {
        if let Some(profile) = profile.filter(|profile| !profile.iso_choices.is_empty()) {
            *self.rng.choice(&profile.iso_choices)
        } else {
            *self.rng.choice(&ISO_CHOICES)
        }
    }

    fn pick_exposure(&mut self, profile: Option<&CameraProfile>) -> (u32, u32) {
        if let Some(profile) = profile.filter(|profile| !profile.exposure_choices.is_empty()) {
            *self.rng.choice(&profile.exposure_choices)
        } else {
            *self.rng.choice(&EXPOSURE_CHOICES)
        }
    }

    fn pick_fnumber(&mut self, profile: Option<&CameraProfile>) -> (u32, u32) {
        if let Some(profile) = profile.filter(|profile| !profile.fnumber_choices.is_empty()) {
            *self.rng.choice(&profile.fnumber_choices)
        } else {
            let fnumber = *self.rng.choice(&FNUMBER_CHOICES);
            (fnumber, 100)
        }
    }

    fn rand_serial(&mut self, prefix: String) -> String {
        const ALPHABET: &[u8] = b"0123456789ABCDEF";
        let mut out = prefix;
        while out.len() < 8 {
            let index = self.rng.below(ALPHABET.len() as u64) as usize;
            out.push(char::from(ALPHABET[index]));
        }
        out
    }
}

fn local_offset<F>(datetime: NaiveDateTime, mut lookup: F) -> String
where
    F: FnMut(NaiveDateTime) -> Option<FixedOffset>,
{
    lookup(datetime)
        .or_else(|| {
            [1_i64, 3, 6]
                .into_iter()
                .find_map(|hours| lookup(datetime - Duration::hours(hours)))
        })
        .map_or_else(
            || format_offset(Local::now().offset().fix().local_minus_utc()),
            |value| format_offset(value.local_minus_utc()),
        )
}

fn parse_offset_seconds(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.len() != 6 || (bytes[0] != b'+' && bytes[0] != b'-') || bytes[3] != b':' {
        return None;
    }
    let hours: i32 = text.get(1..3)?.parse().ok()?;
    let minutes: i32 = text.get(4..6)?.parse().ok()?;
    let seconds = hours * 3600 + minutes * 60;
    Some(if bytes[0] == b'-' { -seconds } else { seconds })
}

/// Backwards-compatible wrapper: fresh randomized metadata.
#[must_use]
pub fn randomize_metadata(
    seed: Option<u64>,
    profile: Option<&CameraProfile>,
    image_time: Option<&ImageTime>,
) -> Metadata {
    MetadataRandomizer::new(seed, profile).randomize(image_time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_offset_falls_back_before_a_gap() {
        let gap = NaiveDate::from_ymd_opt(2024, 10, 6)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        let pre_gap = FixedOffset::east_opt(10 * 3600).unwrap();
        let offset = local_offset(gap, |candidate| {
            (candidate < gap - Duration::minutes(30)).then_some(pre_gap)
        });
        assert_eq!(offset, "+10:00");
    }
}
