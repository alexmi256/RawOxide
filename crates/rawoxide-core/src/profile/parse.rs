//! JSON parsing helpers for camera profiles.

use std::collections::BTreeMap;

use serde_json::Value;

use super::model::{Lens, ProfileError};

pub(super) fn string_or(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: &str,
) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}

pub(super) fn parse_version(
    value: Option<&Value>,
    default: [u8; 4],
) -> Result<[u8; 4], ProfileError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let items = value
        .as_array()
        .ok_or_else(|| ProfileError::Invalid("dng_version must be a list".to_owned()))?;
    if items.len() != 4 {
        return Err(ProfileError::Invalid(
            "dng_version must have four bytes".to_owned(),
        ));
    }
    let mut out = [0_u8; 4];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = u8::try_from(as_u64(item, "dng_version")?).unwrap_or(0);
    }
    Ok(out)
}

pub(super) fn parse_matrix(
    value: Option<&Value>,
    default: [(i32, u32); 9],
) -> Result<[(i32, u32); 9], ProfileError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let items = value
        .as_array()
        .ok_or_else(|| ProfileError::Invalid("color matrix must be a list".to_owned()))?;
    if items.len() != 9 {
        return Err(ProfileError::Invalid(
            "color matrix needs nine rationals".to_owned(),
        ));
    }
    let mut out = default;
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = pair_i32(item, "color_matrix")?;
    }
    Ok(out)
}

pub(super) fn pair_i32(value: &Value, name: &str) -> Result<(i32, u32), ProfileError> {
    let items = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a two-item list")))?;
    let numerator = as_i64(&items[0], name)?;
    let denominator = as_i64(&items[1], name)?;
    if denominator <= 0 {
        return Err(ProfileError::Invalid(format!(
            "{name} denominators must be positive"
        )));
    }
    Ok((
        i32::try_from(numerator).unwrap_or(0),
        u32::try_from(denominator).unwrap_or(1),
    ))
}

pub(super) fn pair_usize(value: &Value, name: &str) -> Result<(usize, usize), ProfileError> {
    let items = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a two-item list")))?;
    Ok((
        usize::try_from(as_u64(&items[0], name)?).unwrap_or(0),
        usize::try_from(as_u64(&items[1], name)?).unwrap_or(0),
    ))
}

pub(super) fn pair_or(
    value: Option<&Value>,
    default: (u32, u32),
) -> Result<(u32, u32), ProfileError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let items = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or_else(|| ProfileError::Invalid("pair must have two values".to_owned()))?;
    Ok((
        u32::try_from(as_u64(&items[0], "pair")?).unwrap_or(0),
        u32::try_from(as_u64(&items[1], "pair")?).unwrap_or(0),
    ))
}

pub(super) fn rationals(value: &Value, name: &str) -> Result<Vec<(u32, u32)>, ProfileError> {
    let items = value
        .as_array()
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a list")))?;
    items
        .iter()
        .map(|item| {
            let fields = item
                .as_array()
                .filter(|fields| fields.len() == 2)
                .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a two-item list")))?;
            let numerator = as_u64(&fields[0], name)?;
            let denominator = as_u64(&fields[1], name)?;
            if denominator == 0 {
                return Err(ProfileError::Invalid(format!(
                    "{name} denominators must be positive"
                )));
            }
            Ok((
                u32::try_from(numerator).unwrap_or(u32::MAX),
                u32::try_from(denominator).unwrap_or(1),
            ))
        })
        .collect()
}

pub(super) fn parse_lenses(value: Option<&Value>) -> Result<Vec<Lens>, ProfileError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| ProfileError::Invalid("lenses must be a list".to_owned()))?;
    items
        .iter()
        .map(|item| {
            let fields = item
                .as_array()
                .filter(|fields| fields.len() >= 4)
                .ok_or_else(|| ProfileError::Invalid("lens needs four fields".to_owned()))?;
            Ok(Lens {
                model: fields[0].as_str().unwrap_or_default().to_owned(),
                min_mm: fields[1].as_f64().unwrap_or(0.0),
                max_mm: fields[2].as_f64().unwrap_or(0.0),
                max_aperture: fields[3].as_f64().unwrap_or(0.0),
                make: fields
                    .get(4)
                    .and_then(Value::as_str)
                    .filter(|make| !make.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub(super) fn parse_u32_list(value: Option<&Value>, name: &str) -> Result<Vec<u32>, ProfileError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a list")))?
        .iter()
        .map(|item| Ok(u32::try_from(as_u64(item, name)?).unwrap_or(0)))
        .collect()
}

pub(super) fn parse_u16_list(value: Option<&Value>, name: &str) -> Result<Vec<u16>, ProfileError> {
    Ok(parse_u32_list(value, name)?
        .into_iter()
        .map(|item| u16::try_from(item).unwrap_or(u16::MAX))
        .collect())
}

pub(super) fn parse_rational_list(
    value: Option<&Value>,
    name: &str,
) -> Result<Vec<(u32, u32)>, ProfileError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    rationals(value, name)
}

pub(super) fn parse_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
    )
}

pub(super) fn parse_subprofiles(
    value: Option<&Value>,
) -> Result<BTreeMap<String, (usize, usize)>, ProfileError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| ProfileError::Invalid("subprofiles must be a dict".to_owned()))?;
    let mut out = BTreeMap::new();
    for (name, geometry) in object {
        let width = geometry
            .get("width")
            .and_then(Value::as_u64)
            .ok_or_else(|| ProfileError::Invalid(format!("subprofile {name:?} needs width")))?;
        let height = geometry
            .get("height")
            .and_then(Value::as_u64)
            .ok_or_else(|| ProfileError::Invalid(format!("subprofile {name:?} needs height")))?;
        if width == 0 || height == 0 {
            return Err(ProfileError::Invalid(format!(
                "subprofile {name:?} needs positive width/height"
            )));
        }
        out.insert(
            name.clone(),
            (
                usize::try_from(width).unwrap_or(0),
                usize::try_from(height).unwrap_or(0),
            ),
        );
    }
    Ok(out)
}

pub(super) fn parse_optional_pair(
    value: Option<&Value>,
    name: &str,
) -> Result<Option<(i32, i32)>, ProfileError> {
    value.map(|value| pair_signed(value, name)).transpose()
}

pub(super) fn pair_signed(value: &Value, name: &str) -> Result<(i32, i32), ProfileError> {
    let items = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be a two-item list")))?;
    let low = as_i64(&items[0], name)?;
    let high = as_i64(&items[1], name)?;
    Ok((
        i32::try_from(low).unwrap_or(0),
        i32::try_from(high).unwrap_or(0),
    ))
}

pub(super) fn parse_optional_float_pair(
    value: Option<&Value>,
    name: &str,
) -> Result<Option<(f64, f64)>, ProfileError> {
    let Some(value) = value else { return Ok(None) };
    let items = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must have two values")))?;
    let low = items[0].as_f64().unwrap_or(0.0);
    let high = items[1].as_f64().unwrap_or(0.0);
    if !(low > 0.0 && low <= high) {
        return Err(ProfileError::Invalid(format!(
            "{name} must be 0 < low <= high"
        )));
    }
    Ok(Some((low, high)))
}

pub(super) fn optional_usize(value: Option<&Value>) -> Result<Option<usize>, ProfileError> {
    let Some(value) = value else { return Ok(None) };
    Ok(Some(usize::try_from(as_u64(value, "value")?).unwrap_or(0)))
}

pub(super) fn u16_or(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: u16,
) -> Result<u16, ProfileError> {
    object.get(key).map_or(Ok(default), |value| {
        Ok(u16::try_from(as_u64(value, key)?).unwrap_or(default))
    })
}

pub(super) fn u32_or(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: u32,
) -> Result<u32, ProfileError> {
    object.get(key).map_or(Ok(default), |value| {
        Ok(u32::try_from(as_u64(value, key)?).unwrap_or(default))
    })
}

pub(super) fn u8_or(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: u8,
) -> Result<u8, ProfileError> {
    object.get(key).map_or(Ok(default), |value| {
        Ok(u8::try_from(as_u64(value, key)?).unwrap_or(default))
    })
}

pub(super) fn as_u64(value: &Value, name: &str) -> Result<u64, ProfileError> {
    value
        .as_u64()
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be an integer")))
}

pub(super) fn as_i64(value: &Value, name: &str) -> Result<i64, ProfileError> {
    value
        .as_i64()
        .ok_or_else(|| ProfileError::Invalid(format!("{name} must be an integer")))
}

pub(super) fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, strip_nulls(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

pub(super) fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.as_chunks::<2>().0 {
        let high = hex_digit(chunk[0])?;
        let low = hex_digit(chunk[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
