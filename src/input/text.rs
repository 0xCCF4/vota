//! Rules for free text.

use unicode_normalization::UnicodeNormalization as _;

use super::characters::is_forbidden;

/// Why a single value was rejected; the caller attaches the field name.
pub(super) type Rejection = &'static str;

/// Required single-line text: NFC-normalised, trimmed, non-empty, no
/// control, invisible or unassigned characters ([`is_forbidden`]), at most
/// [`MAX_MARKS_PER_BASE`] combining marks per base character, at most
/// `max_chars` characters.
pub(super) fn single_line(raw: String, max_chars: usize) -> Result<String, Rejection> {
    let text: String = raw.nfc().collect();
    let text = text.trim();
    if text.is_empty() {
        return Err("must not be empty");
    }
    if text.chars().any(is_forbidden) {
        return Err("contains control, invisible or unassigned characters");
    }
    if has_too_many_marks(text) {
        return Err("too many combining marks");
    }
    if text.chars().count() > max_chars {
        return Err("too long");
    }
    Ok(text.to_owned())
}

/// Required multi-line text: like [`single_line`], except that line breaks
/// (`\r\n` and lone `\r` are normalised to `\n` first) and tabs are allowed.
pub(super) fn multi_line(raw: String, max_chars: usize) -> Result<String, Rejection> {
    let text: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .nfc()
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Err("must not be empty");
    }
    if text
        .chars()
        .any(|c| !matches!(c, '\n' | '\t') && is_forbidden(c))
    {
        return Err("contains control, invisible or unassigned characters");
    }
    if has_too_many_marks(text) {
        return Err("too many combining marks");
    }
    if text.chars().count() > max_chars {
        return Err("too long");
    }
    Ok(text.to_owned())
}

/// The most combining marks (general categories Mn, Mc, Me) one base
/// character may carry, counted after NFC normalisation. Real scripts need
/// at most three or so; stacks of marks ("Zalgo text") only obscure.
const MAX_MARKS_PER_BASE: usize = 4;

/// Whether some base character carries more than [`MAX_MARKS_PER_BASE`]
/// combining marks in a row.
fn has_too_many_marks(text: &str) -> bool {
    use icu_properties::CodePointMapData;
    use icu_properties::props::GeneralCategory as Gc;
    let categories = CodePointMapData::<Gc>::new();
    let mut run = 0;
    for c in text.chars() {
        if matches!(
            categories.get(c),
            Gc::NonspacingMark | Gc::SpacingMark | Gc::EnclosingMark
        ) {
            run += 1;
            if run > MAX_MARKS_PER_BASE {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}
