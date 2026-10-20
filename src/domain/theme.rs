//! Themes and Theme Tweaks: the names a Ballot refers to them by, and the
//! accent colour. Which Themes and logos exist is the Theme registry's
//! business ([`ThemeCatalog`]); this module only knows the rules.

use crate::input::Sanitized;

/// The Theme a Ballot uses when the admin names none, and the one it falls
/// back to if its Theme has gone (removed from the theme folder).
pub const DEFAULT_THEME: &str = "classic";

/// The name of a Theme that existed when the admin chose it: lowercase ASCII
/// letters, digits and `-`, starting with a letter or digit, 1..=64
/// characters ([`is_valid_name`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeName(String);

impl ThemeName {
    pub(crate) fn new(name: String, _: Sanitized) -> Self {
        Self(name)
    }

    /// The default Theme ([`DEFAULT_THEME`]), which always exists.
    pub fn default_theme() -> Self {
        Self(DEFAULT_THEME.to_owned())
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(name: String) -> Self {
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of a logo that existed when the admin chose it; the same form
/// as a [`ThemeName`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoName(String);

impl LogoName {
    pub(crate) fn new(name: String, _: Sanitized) -> Self {
        Self(name)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(name: String) -> Self {
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Maximum length of a Theme or logo name.
pub const MAX_NAME_CHARS: usize = 64;

/// Whether `name` is a well-formed Theme or logo name:
/// `^[a-z0-9][a-z0-9-]{0,63}$`. Such a name can never contain a path
/// separator, `.` or `..`, so it is safe as a file or directory name; it is
/// still only ever looked up among the registry's known names, never joined
/// onto a path.
pub fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
    !bytes.is_empty()
        && bytes.len() <= MAX_NAME_CHARS
        && bytes[0] != b'-'
        && bytes.iter().all(allowed)
}

/// Which Themes and logos exist: the Theme registry, as validation sees it.
pub trait ThemeCatalog {
    /// Whether a Theme of this name exists (built-in or in the theme folder).
    fn has_theme(&self, name: &str) -> bool;
    /// Whether a logo of this name exists in the logo folder.
    fn has_logo(&self, name: &str) -> bool;
}

/// An accent colour Theme Tweak: `#rrggbb`, lowercase. Only ever applied as
/// the value of the CSS custom property `--accent`, never as raw CSS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccentColor(String);

impl AccentColor {
    /// `hex`: `#` and six hex digits, any case ([`Self::parse`] checks it).
    pub(crate) fn new(hex: &str, _: Sanitized) -> Option<Self> {
        Self::parse(hex)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones (its schema checks the form, too).
    pub(crate) fn from_store(hex: String) -> Self {
        Self(hex)
    }

    /// `^#[0-9a-fA-F]{6}$`, lowercased.
    fn parse(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(hex.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Black or white, whichever contrasts more with this colour (WCAG 2
    /// contrast ratio), for text on it: `#000000` or `#ffffff`.
    pub fn contrast_text(&self) -> &'static str {
        let channel = |i: usize| {
            let value = u8::from_str_radix(&self.0[i..i + 2], 16).unwrap_or(0);
            let c = f64::from(value) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance = 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
        let on_black = (luminance + 0.05) / 0.05;
        let on_white = 1.05 / (luminance + 0.05);
        if on_black > on_white {
            "#000000"
        } else {
            "#ffffff"
        }
    }
}
