//! Debug-sidecar document helpers.
//!
//! The JSON shape follows the Python `stegodng.debugmeta` document: a
//! re-read tag inventory per IFD, a hashed preview-JPEG record, and a full
//! camera-profile snapshot with binary blobs described by size + CRC32.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use rawhide_core::{
    CameraProfile, DngReader, FidelityConfig,
    profile::{AdobeProfile, GainMapConfig, MakernoteConfig},
};
use serde_json::{Value, json};
use tiff_reader::{ByteOrder, Ifd, Tag, TagValue};

/// Re-read a written DNG and describe its IFDs and preview JPEG.
///
/// # Errors
///
/// Returns an error when the file cannot be opened or parsed as TIFF.
pub fn file_inventory(path: &Path) -> anyhow::Result<Value> {
    let reader = DngReader::open(path)?;
    let ifd0 = reader.ifd0()?;
    let mut ifd0_json = ifd_snapshot(ifd0, "tiff");
    patch_sub_ifd_tag(&mut ifd0_json, &reader)?;
    let mut document = json!({
        "ifd0": ifd0_json,
    });
    let subifds = reader
        .raw_ifds()?
        .iter()
        .map(|ifd| ifd_snapshot(ifd, "tiff"))
        .collect::<Vec<_>>();
    document["subifds"] = Value::Array(subifds);
    if let Some(exif) = reader.exif_ifd()? {
        document["exif"] = ifd_snapshot(&exif, "exif");
    }
    if let Some(gps) = reader.gps_ifd()? {
        document["gps"] = ifd_snapshot(&gps, "gps");
    }
    if let Some(preview) = preview_blob(path, ifd0) {
        document["preview_jpeg"] = preview;
    }
    Ok(document)
}

fn ifd_snapshot(ifd: &Ifd, namespace: &str) -> Value {
    let mut tags = ifd.tags().iter().collect::<Vec<_>>();
    tags.sort_by_key(|tag| tag.code);
    Value::Array(
        tags.into_iter()
            .map(|tag| tag_snapshot(tag, namespace))
            .collect(),
    )
}

fn patch_sub_ifd_tag(ifd0: &mut Value, reader: &DngReader) -> anyhow::Result<()> {
    let Some(tags) = ifd0.as_array_mut() else {
        return Ok(());
    };
    let Some(tag) = tags.iter_mut().find(|tag| tag["tag"] == 330) else {
        return Ok(());
    };
    let offsets = reader.sub_ifd_offsets()?;
    let original = tag["type"].as_str().unwrap_or_default();
    let tiff_type = match original {
        "LONG" => "LONG",
        "LONG8" => "LONG8",
        "IFD8" => "IFD8",
        _ => "IFD",
    };
    tag["type"] = Value::String(tiff_type.to_owned());
    tag["count"] = json!(offsets.len());
    tag["value"] = single_or_array(json!(offsets));
    Ok(())
}

fn tag_snapshot(tag: &Tag, namespace: &str) -> Value {
    let encoded = tag.value.encode(ByteOrder::LittleEndian);
    let value = if matches!(tag.code, 50_781 | 51_111) && encoded.len() == 16 {
        Value::String(hex(&encoded))
    } else if encoded.len() > 256 {
        bytes_json(&encoded)
    } else {
        decode_tag_value(&tag.value)
    };
    json!({
        "tag": tag.code,
        "name": tag_name(tag.code, namespace),
        "type": type_name(tag.tag_type),
        "count": tag.count,
        "value": value,
    })
}

fn decode_tag_value(value: &TagValue) -> Value {
    match value {
        TagValue::Ascii(text) => Value::String(text.trim_end_matches('\0').to_owned()),
        TagValue::Byte(values) => single_or_array(json!(values)),
        TagValue::Undefined(values) => {
            if values.len() <= 32 {
                Value::String(hex(values))
            } else {
                bytes_json(values)
            }
        }
        TagValue::Short(values) => single_or_array(json!(values)),
        TagValue::Long(values) => single_or_array(json!(values)),
        TagValue::Rational(values) => single_or_array(json!(values)),
        TagValue::SByte(values) => single_or_array(json!(values)),
        TagValue::SShort(values) => single_or_array(json!(values)),
        TagValue::SLong(values) => single_or_array(json!(values)),
        TagValue::SRational(values) => single_or_array(json!(values)),
        TagValue::Float(values) => single_or_array(json!(values)),
        TagValue::Double(values) => single_or_array(json!(values)),
        TagValue::Long8(values) => single_or_array(json!(values)),
        TagValue::SLong8(values) => single_or_array(json!(values)),
    }
}

fn single_or_array(value: Value) -> Value {
    if let Value::Array(items) = &value
        && items.len() == 1
    {
        return items[0].clone();
    }
    value
}

fn type_name(tag_type: tiff_reader::TagType) -> String {
    let text = format!("{tag_type:?}");
    if let Some(code) = text
        .strip_prefix("Unknown(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return format!("type_{code}");
    }
    text.to_uppercase()
}

fn preview_blob(path: &Path, ifd0: &Ifd) -> Option<Value> {
    let offsets = tag_u64_vec(ifd0, 273).or_else(|| tag_u64_vec(ifd0, 324))?;
    let counts = tag_u64_vec(ifd0, 279).or_else(|| tag_u64_vec(ifd0, 325))?;
    if offsets.len() != counts.len() || offsets.is_empty() {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut hasher = crc32fast::Hasher::new();
    let mut total = 0_u64;
    for (offset, count) in offsets.into_iter().zip(counts) {
        if count == 0 {
            continue;
        }
        let length = usize::try_from(count).ok()?;
        let mut buffer = vec![0_u8; length];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        hasher.update(&buffer);
        total = total.saturating_add(count);
    }
    Some(json!({
        "bytes": total,
        "crc32": format!("{:08x}", hasher.finalize()),
        "strips": offsets_len(ifd0),
    }))
}

fn offsets_len(ifd0: &Ifd) -> usize {
    tag_u64_vec(ifd0, 273)
        .or_else(|| tag_u64_vec(ifd0, 324))
        .map_or(0, |values| values.len())
}

fn tag_u64_vec(ifd: &Ifd, code: u16) -> Option<Vec<u64>> {
    match &ifd.tag(code)?.value {
        TagValue::Short(values) => Some(values.iter().map(|value| u64::from(*value)).collect()),
        TagValue::Long(values) => Some(values.iter().map(|value| u64::from(*value)).collect()),
        TagValue::Long8(values) => Some(values.clone()),
        _ => None,
    }
}

/// Full static profile snapshot, with binary blobs hashed like Python.
pub fn profile_json(profile: &CameraProfile) -> Value {
    json!({
        "make": &profile.make,
        "model": &profile.model,
        "unique_camera_model": &profile.unique_camera_model,
        "software": &profile.software,
        "dng_version": bytes_json(&profile.dng_version),
        "dng_backward": bytes_json(&profile.dng_backward),
        "color_matrix1": &profile.color_matrix1,
        "color_matrix2": &profile.color_matrix2,
        "calib_illuminant1": profile.calib_illuminant1,
        "calib_illuminant2": profile.calib_illuminant2,
        "baseline_noise": profile.baseline_noise,
        "baseline_sharpness": profile.baseline_sharpness,
        "linear_response_limit": profile.linear_response_limit,
        "shadow_scale": profile.shadow_scale,
        "black_level_ref": profile.black_level_ref,
        "opcode_list3": bytes_json(&profile.opcode_list3),
        "gainmap": profile.gainmap.as_ref().map(gainmap_json),
        "fidelity": profile.fidelity.as_ref().map(fidelity_json),
        "slug": &profile.slug,
        "display_name": &profile.display_name,
        "native_width": profile.native_width,
        "native_height": profile.native_height,
        "suggested_bit_depth": profile.suggested_bit_depth,
        "cfa_pattern": &profile.cfa_pattern,
        "crop_factor": profile.crop_factor,
        "lenses": if profile.lenses.is_empty() {
            Value::Null
        } else {
            json!(profile
                .lenses
                .iter()
                .map(|lens| json!([
                    &lens.model,
                    lens.min_mm,
                    lens.max_mm,
                    lens.max_aperture,
                    lens.make.as_deref().unwrap_or(""),
                ]))
                .collect::<Vec<_>>())
        },
        "iso_choices": pool_json(&profile.iso_choices),
        "exposure_choices": pool_json(&profile.exposure_choices),
        "fnumber_choices": pool_json(&profile.fnumber_choices),
        "metering_choices": pool_json(&profile.metering_choices),
        "exposure_program_choices": pool_json(&profile.exposure_program_choices),
        "serial_prefixes": &profile.serial_prefixes,
        "subprofiles": &profile.subprofiles,
        "calibration_source": &profile.calibration_source,
    })
}

fn pool_json<T: serde::Serialize>(values: &[T]) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        json!(values)
    }
}

fn fidelity_json(fidelity: &FidelityConfig) -> Value {
    json!({
        "preview_size": fidelity.preview_size,
        "orientations": &fidelity.orientations,
        "software_variants": &fidelity.software_variants,
        "date_range": &fidelity.date_range,
        "date_window_days": fidelity.date_window_days,
        "suggested_mode": &fidelity.suggested_mode,
        "asn_r": fidelity.asn_r,
        "asn_b": fidelity.asn_b,
        "baseline_exposure": fidelity.baseline_exposure,
        "subject_distance": &fidelity.subject_distance,
        "subject_distance_range": &fidelity.subject_distance_range,
        "exposure_bias": &fidelity.exposure_bias,
        "flash": fidelity.flash,
        "emit_xmp": fidelity.emit_xmp,
        "emit_lens": fidelity.emit_lens,
        "emit_serials": fidelity.emit_serials,
        "emit_gps": fidelity.emit_gps,
        "exif_allow": &fidelity.exif_allow,
        "tag_conventions": &fidelity.tag_conventions,
        "raw_digests": fidelity.raw_digests,
        "makernote": fidelity.makernote.as_ref().map(makernote_json),
        "subsec_digits": fidelity.subsec_digits,
        "tile": fidelity.tile,
        "black_levels": &fidelity.black_levels,
        "white_level": fidelity.white_level,
        "max_planes": fidelity.max_planes,
        "level_policy": &fidelity.level_policy,
        "crop_margin": fidelity.crop_margin,
        "bayer_green_split": fidelity.bayer_green_split,
        "sensor_sizes": &fidelity.sensor_sizes,
        "prefer_native": fidelity.prefer_native,
        "noise_s": fidelity.noise_s,
        "noise_o": fidelity.noise_o,
        "adobe": fidelity.adobe.as_ref().map(adobe_json),
    })
}

fn makernote_json(config: &MakernoteConfig) -> Value {
    json!({
        "magic": bytes_json(&config.magic),
        "prefix": bytes_json(&config.prefix),
        "length": [config.length.0, config.length.1],
    })
}

fn adobe_json(adobe: &AdobeProfile) -> Value {
    json!({
        "forward_matrix1": adobe.forward_matrix1,
        "forward_matrix2": adobe.forward_matrix2,
        "name": &adobe.name,
        "calibration_signature": &adobe.calibration_signature,
        "copyright": &adobe.copyright,
        "embed_policy": adobe.embed_policy,
        "hue_sat_map_dims": adobe.hue_sat_map_dims,
        "hue_sat_map_data1": adobe.hue_sat_map_data1.as_deref().map(f32_blob),
        "hue_sat_map_data2": adobe.hue_sat_map_data2.as_deref().map(f32_blob),
        "look_table_dims": adobe.look_table_dims,
        "look_table_data": adobe.look_table_data.as_deref().map(f32_blob),
    })
}

fn gainmap_json(config: &GainMapConfig) -> Value {
    let plane_len = config.rows.saturating_mul(config.cols);
    let planes = (0..4_usize)
        .map(|phase| {
            let start = phase.saturating_mul(plane_len);
            let end = start.saturating_add(plane_len).min(config.planes.len());
            let flat = &config.planes[start.min(end)..end];
            (0..config.rows)
                .map(|row| {
                    let row_start = row.saturating_mul(config.cols);
                    let row_end = row_start.saturating_add(config.cols).min(flat.len());
                    flat[row_start.min(row_end)..row_end].to_vec()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    json!({
        "planes": planes,
        "jitter": config.jitter,
        "clip": [config.clip.0, config.clip.1],
    })
}

fn f32_blob(values: &[f32]) -> Value {
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes_json(&bytes)
}

pub fn bytes_json(bytes: &[u8]) -> Value {
    let mut object = json!({
        "bytes": bytes.len(),
        "crc32": format!("{:08x}", crc32fast::hash(bytes)),
    });
    if bytes.len() <= 32 {
        object["hex"] = Value::String(hex(bytes));
    }
    object
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn tag_name(code: u16, namespace: &str) -> String {
    let name = match namespace {
        "gps" => gps_tag_name(code),
        "exif" => exif_tag_name(code),
        _ => tiff_tag_name(code),
    };
    name.map_or_else(|| format!("tag_{code}"), str::to_owned)
}

const fn tiff_tag_name(code: u16) -> Option<&'static str> {
    Some(match code {
        254 => "NewSubfileType",
        256 => "ImageWidth",
        257 => "ImageLength",
        258 => "BitsPerSample",
        259 => "Compression",
        262 => "PhotometricInterpretation",
        270 => "ImageDescription",
        271 => "Make",
        272 => "Model",
        273 => "StripOffsets",
        274 => "Orientation",
        277 => "SamplesPerPixel",
        278 => "RowsPerStrip",
        279 => "StripByteCounts",
        282 => "XResolution",
        283 => "YResolution",
        284 => "PlanarConfiguration",
        296 => "ResolutionUnit",
        297 => "PageNumber",
        305 => "Software",
        306 => "DateTime",
        317 => "Predictor",
        322 => "TileWidth",
        323 => "TileLength",
        324 => "TileOffsets",
        325 => "TileByteCounts",
        330 => "SubIFDs",
        529 => "YCbCrCoefficients",
        530 => "YCbCrSubSampling",
        531 => "YCbCrPositioning",
        532 => "ReferenceBlackWhite",
        700 => "XMP",
        33421 => "CFARepeatPatternDim",
        33422 => "CFAPattern",
        34665 => "ExifTag",
        34853 => "GPSTag",
        50706 => "DNGVersion",
        50707 => "DNGBackwardVersion",
        50708 => "UniqueCameraModel",
        50710 => "CFAPlaneColor",
        50711 => "CFALayout",
        50713 => "BlackLevelRepeatDim",
        50714 => "BlackLevel",
        50717 => "WhiteLevel",
        50718 => "DefaultScale",
        50719 => "DefaultCropOrigin",
        50720 => "DefaultCropSize",
        50721 => "ColorMatrix1",
        50722 => "ColorMatrix2",
        50727 => "AnalogBalance",
        50728 => "AsShotNeutral",
        50730 => "BaselineExposure",
        50731 => "BaselineNoise",
        50732 => "BaselineSharpness",
        50733 => "BayerGreenSplit",
        50734 => "LinearResponseLimit",
        50735 => "CameraSerialNumber",
        50736 => "LensInfo",
        50738 => "AntiAliasStrength",
        50739 => "ShadowScale",
        50741 => "MakerNoteSafety",
        50778 => "CalibrationIlluminant1",
        50779 => "CalibrationIlluminant2",
        50780 => "BestQualityScale",
        50781 => "RawDataUniqueID",
        50829 => "ActiveArea",
        50932 => "ProfileCalibrationSignature",
        50936 => "ProfileName",
        50937 => "ProfileHueSatMapDims",
        50938 => "ProfileHueSatMapData1",
        50939 => "ProfileHueSatMapData2",
        50941 => "ProfileEmbedPolicy",
        50942 => "ProfileCopyright",
        50964 => "ForwardMatrix1",
        50965 => "ForwardMatrix2",
        50981 => "ProfileLookTableDims",
        50982 => "ProfileLookTableData",
        51009 => "OpcodeList2",
        51022 => "OpcodeList3",
        51041 => "NoiseProfile",
        51111 => "NewRawImageDigest",
        51125 => "DefaultUserCrop",
        _ => return None,
    })
}

const fn exif_tag_name(code: u16) -> Option<&'static str> {
    Some(match code {
        33434 => "ExposureTime",
        33437 => "FNumber",
        34850 => "ExposureProgram",
        34855 => "ISOSpeedRatings",
        36867 => "DateTimeOriginal",
        36868 => "DateTimeDigitized",
        36880 => "OffsetTime",
        36881 => "OffsetTimeOriginal",
        36882 => "OffsetTimeDigitized",
        37377 => "ShutterSpeedValue",
        37378 => "ApertureValue",
        37379 => "BrightnessValue",
        37380 => "ExposureBiasValue",
        37381 => "MaxApertureValue",
        37382 => "SubjectDistance",
        37383 => "MeteringMode",
        37384 => "LightSource",
        37385 => "Flash",
        37386 => "FocalLength",
        37393 => "ImageNumber",
        37500 => "MakerNote",
        37520 => "SubsecTime",
        37521 => "SubsecTimeOriginal",
        37522 => "SubsecTimeDigitized",
        40965 => "InteropOffset",
        41730 => "CFAPattern",
        41987 => "WhiteBalance",
        41988 => "DigitalZoomRatio",
        41989 => "FocalLengthIn35mmFilm",
        41990 => "SceneCaptureType",
        41991 => "GainControl",
        41992 => "Contrast",
        41993 => "Saturation",
        41994 => "Sharpness",
        41996 => "SubjectDistanceRange",
        42016 => "ImageUniqueID",
        42033 => "BodySerialNumber",
        42034 => "LensSpecification",
        42035 => "LensMake",
        42036 => "LensModel",
        42037 => "LensSerialNumber",
        _ => return None,
    })
}

const fn gps_tag_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "GPSVersionID",
        1 => "GPSLatitudeRef",
        2 => "GPSLatitude",
        3 => "GPSLongitudeRef",
        4 => "GPSLongitude",
        5 => "GPSAltitudeRef",
        6 => "GPSAltitude",
        7 => "GPSTimeStamp",
        8 => "GPSSatellites",
        9 => "GPSStatus",
        10 => "GPSMeasureMode",
        11 => "GPSDOP",
        12 => "GPSSpeedRef",
        13 => "GPSSpeed",
        14 => "GPSTrackRef",
        15 => "GPSTrack",
        16 => "GPSImgDirectionRef",
        17 => "GPSImgDirection",
        18 => "GPSMapDatum",
        19 => "GPSDestLatitudeRef",
        20 => "GPSDestLatitude",
        21 => "GPSDestLongitudeRef",
        22 => "GPSDestLongitude",
        23 => "GPSDestBearingRef",
        24 => "GPSDestBearing",
        25 => "GPSDestDistanceRef",
        26 => "GPSDestDistance",
        27 => "GPSProcessingMethod",
        28 => "GPSAreaInformation",
        29 => "GPSDateStamp",
        30 => "GPSDifferential",
        31 => "GPSHPositioningError",
        _ => return None,
    })
}
