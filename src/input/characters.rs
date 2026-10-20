//! The characters free text may not contain. Shared by the server's text
//! rules (`text`) and by the CLI, which escapes them when it prints text it
//! didn't write itself.

/// Whether `c` may not appear in free text, so a listing of text stored
/// before these rules escapes it instead of printing it.
pub fn is_unprintable(c: char) -> bool {
    is_forbidden(c) || c == '\n' || c == '\t'
}

/// Characters free text may not contain: controls (Cc), format characters
/// (Cf: bidi controls, zero-width characters, invisible operators, tags,
/// prepended number signs, …), private use (Co), unassigned code points and
/// noncharacters (Cn), and the invisible characters of other categories
/// ([`is_invisible_otherwise`]).
pub(super) fn is_forbidden(c: char) -> bool {
    use icu_properties::CodePointMapData;
    use icu_properties::props::GeneralCategory as Gc;
    let category = CodePointMapData::<Gc>::new().get(c);
    matches!(
        category,
        Gc::Control | Gc::Format | Gc::PrivateUse | Gc::Unassigned
    ) || is_noncharacter(c)
        || is_invisible_otherwise(c)
}

/// The 66 noncharacters: U+FDD0..U+FDEF and the last two code points of
/// every plane (all unassigned, Cn, already; named for clarity).
fn is_noncharacter(c: char) -> bool {
    matches!(c, '\u{FDD0}'..='\u{FDEF}') || (c as u32) & 0xFFFE == 0xFFFE
}

/// Invisible or blank characters outside the categories refused as a whole:
/// fillers that render as nothing, separators, variation selectors and the
/// like.
fn is_invisible_otherwise(c: char) -> bool {
    matches!(
        c,
        '\u{034F}'                 // combining grapheme joiner (Mn)
        | '\u{115F}' | '\u{1160}'   // Hangul fillers (Lo)
        | '\u{17B4}' | '\u{17B5}'   // Khmer inherent vowels (Mn)
        | '\u{180B}'..='\u{180D}'   // Mongolian variation selectors (Mn)
        | '\u{180F}'               // Mongolian variation selector four (Mn)
        | '\u{2028}' | '\u{2029}'   // line/paragraph separator (Zl, Zp)
        | '\u{2800}'               // Braille pattern blank (So)
        | '\u{3164}'               // Hangul filler (Lo)
        | '\u{FE00}'..='\u{FE0F}'   // variation selectors (Mn)
        | '\u{FFA0}'               // halfwidth Hangul filler (Lo)
        | '\u{FFFC}'               // object replacement character (So)
        | '\u{E0100}'..='\u{E01EF}' // variation selectors supplement (Mn)
    )
}
