//! Theme and logo names, from admin requests and asset paths, and app asset
//! versions.

use untrusted_value::{SanitizeWith as _, UntrustedValue};

use super::Sanitized;
use super::text::Rejection;
use crate::domain::theme::{AccentColor, LogoName, ThemeCatalog, ThemeName, is_valid_name};

/// A Theme name that is well-formed ([`is_valid_name`]) and names an existing
/// Theme. Checked by exact match: no trimming, no case folding.
pub(super) fn known_theme(raw: String, catalog: &dyn ThemeCatalog) -> Result<ThemeName, Rejection> {
    if !is_valid_name(&raw) {
        return Err("must be a name of lowercase letters, digits and '-' (at most 64)");
    }
    if !catalog.has_theme(&raw) {
        return Err("no Theme of this name");
    }
    Ok(ThemeName::new(raw, Sanitized::PROOF))
}

/// A logo name that is well-formed ([`is_valid_name`]) and names an existing
/// logo. Checked by exact match: no trimming, no case folding.
pub(super) fn known_logo(raw: String, catalog: &dyn ThemeCatalog) -> Result<LogoName, Rejection> {
    if !is_valid_name(&raw) {
        return Err("must be a name of lowercase letters, digits and '-' (at most 64)");
    }
    if !catalog.has_logo(&raw) {
        return Err("no logo of this name");
    }
    Ok(LogoName::new(raw, Sanitized::PROOF))
}

/// The logo named by an asset path segment (`/assets/logos/{name}`), if it
/// is well-formed and exists; anything else is not found.
pub fn logo_asset(raw: UntrustedValue<String>, catalog: &dyn ThemeCatalog) -> Option<LogoName> {
    raw.sanitize_with(|raw| known_logo(raw, catalog)).ok()
}

/// The Theme named by an asset path segment (`/assets/themes/{name}/…`), if
/// it is well-formed and exists; anything else is not found.
pub fn theme_asset(raw: UntrustedValue<String>, catalog: &dyn ThemeCatalog) -> Option<ThemeName> {
    raw.sanitize_with(|raw| known_theme(raw, catalog)).ok()
}

/// An accent colour: `^#[0-9a-fA-F]{6}$` exactly (no trimming, no shorthand,
/// no names), stored lowercase.
pub(super) fn accent_color(raw: String) -> Result<AccentColor, Rejection> {
    AccentColor::new(&raw, Sanitized::PROOF).ok_or("must be a hex colour of the form #rrggbb")
}

/// Whether an app asset's query string (`v=<version>`) asks for `current`
/// exactly. Anything else (none, an old version, other parameters) is not
/// the current version, but no error either.
pub fn is_current_version(raw: UntrustedValue<String>, current: &str) -> bool {
    raw.sanitize_with(|raw| {
        Ok::<_, std::convert::Infallible>(raw.strip_prefix("v=") == Some(current))
    })
    .unwrap_or(false)
}

/// Whether an app asset's version path segment (`/assets/app/{version}/…`)
/// is `current` exactly. Anything else (an old version, any other text) is
/// not the current version, but no error either.
pub fn is_current_version_segment(raw: UntrustedValue<String>, current: &str) -> bool {
    raw.sanitize_with(|raw| Ok::<_, std::convert::Infallible>(raw == current))
        .unwrap_or(false)
}

/// The app module a path segment (`/assets/app/{version}/{file}`) names:
/// one of `modules`' names by exact match (no case folding, no
/// normalisation), or none.
pub fn app_module<'a>(
    raw: UntrustedValue<String>,
    modules: &[(&'a str, &'a str)],
) -> Option<&'a str> {
    raw.sanitize_with(|raw| {
        modules
            .iter()
            .map(|(name, _)| *name)
            .find(|name| *name == raw)
            .ok_or(())
    })
    .ok()
}
