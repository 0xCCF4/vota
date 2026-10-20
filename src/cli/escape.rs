//! Escaping of text the CLI prints but didn't write itself (stored Admin
//! Token labels, a server's answers), so control, bidi and other invisible
//! characters can't steer the terminal.

use crate::input;

/// `text` with every character that free text may not contain (controls,
/// bidi and other invisible ones, newline and tab) written as `\u{…}`.
pub(super) fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if input::is_unprintable(c) {
                format!("\\u{{{:x}}}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// JSON text (as serde_json writes it) in which every character free text
/// may not contain is a `\uXXXX` escape (serde_json escapes only ASCII
/// controls): the same JSON value, safe to print on a terminal.
pub(super) fn escape_json(json: &str) -> String {
    let mut escaped = String::new();
    for c in json.chars() {
        if input::is_unprintable(c) {
            // serde_json writes ASCII controls escaped and keeps the JSON's
            // own newlines and indentation outside strings; anything else
            // unprintable is inside a string: escape it as UTF-16.
            if c == '\n' || c == '\t' {
                escaped.push(c);
                continue;
            }
            for unit in c.encode_utf16(&mut [0; 2]) {
                escaped += &format!("\\u{unit:04x}");
            }
        } else {
            escaped.push(c);
        }
    }
    escaped
}
