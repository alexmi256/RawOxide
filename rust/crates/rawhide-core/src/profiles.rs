//! Embedded camera-profile registry.
//!
//! Mirrors Python `stegodng/profiles/__init__.py`: every `profile.json` is
//! compiled into the binary, parsed lazily, and a corrupt entry is skipped
//! rather than failing the whole registry. The built-in `gfx_100` reference
//! profile is always available.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::profile::{CameraProfile, ProfileError};

pub(crate) mod generated {
    include!("profiles_generated.rs");
}

static CACHE: OnceLock<BTreeMap<String, CameraProfile>> = OnceLock::new();

fn asset_lookup(slug: &str, name: &str) -> Option<&'static [u8]> {
    generated::ASSET_FILES
        .iter()
        .find(|(path, _)| {
            let path = *path;
            path == format!("{slug}/{name}") || path == format!("custom/{slug}/{name}")
        })
        .map(|(_, bytes)| *bytes)
}

fn load_all() -> &'static BTreeMap<String, CameraProfile> {
    CACHE.get_or_init(|| {
        let mut profiles = BTreeMap::new();
        profiles.insert("gfx_100".to_owned(), CameraProfile::default());
        for (slug, json) in generated::PROFILE_FILES {
            if *slug == "gfx_100" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
                continue;
            };
            let assets = |name: &str| asset_lookup(slug, name);
            let Ok(profile) = CameraProfile::from_json(&value, &assets) else {
                continue;
            };
            profiles.insert((*slug).to_owned(), profile);
        }
        profiles
    })
}

/// Sorted slugs of every available `--camera-profile` (base names).
#[must_use]
pub fn list_profiles() -> Vec<&'static str> {
    load_all().keys().map(String::as_str).collect()
}

/// Return the profile for `slug`, or `None` when unknown.
#[must_use]
pub fn get_profile(slug: &str) -> Option<&'static CameraProfile> {
    load_all().get(slug)
}

/// Resolve a `slug[-subprofile]` spec, longest registered slug first.
///
/// # Errors
///
/// Returns [`ProfileError::Invalid`] for unknown slugs or subprofiles.
pub fn resolve(spec: &str) -> Result<(&'static CameraProfile, Option<&'static str>), ProfileError> {
    let profiles = load_all();
    if let Some(profile) = profiles.get(spec) {
        return Ok((profile, None));
    }
    if spec.contains('-') {
        let parts: Vec<&str> = spec.split('-').collect();
        for index in (1..parts.len()).rev() {
            let slug = parts[..index].join("-");
            let subprofile = parts[index..].join("-");
            if let Some(profile) = profiles.get(&slug) {
                if profile.subprofiles.contains_key(&subprofile) {
                    let key = profile
                        .subprofiles
                        .get_key_value(&subprofile)
                        .map(|(name, _)| name.as_str())
                        .unwrap_or_default();
                    return Ok((profile, Some(key)));
                }
                return Err(ProfileError::Invalid(format!(
                    "unknown sub-profile {subprofile:?} for camera {slug:?}: choose from {:?}",
                    profile.subprofiles.keys().collect::<Vec<_>>()
                )));
            }
        }
    }
    Err(ProfileError::Invalid(format!(
        "unknown camera profile {spec:?}: choose from {}",
        list_profiles().join(", ")
    )))
}

/// Profiles embedded in this build.
#[must_use]
pub fn count() -> usize {
    load_all().len()
}
