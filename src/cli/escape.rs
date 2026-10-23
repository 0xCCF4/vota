//! Text the CLI prints but didn't write itself (a server's answers, stored
//! Admin Token labels, flag values it echoes in errors), made safe for a
//! terminal: control, bidi and other invisible characters can't steer it.
//!
//! The three types here are the only shapes such text is printed in. Each
//! is built only by escaping, so every value of them is terminal-safe,
//! wherever it was made:
//!
//! - [`PrintableText`]: one line; every character free text may not
//!   contain (newline and tab included) written as `\u{…}`;
//! - [`PrintableLines`]: the same, but line feeds and tabs kept, for CSV
//!   cells, where a quoted cell may span lines;
//! - [`PrintableJson`]: JSON text in which every such character inside a
//!   string is a `\uXXXX` escape: the same JSON value.

use std::fmt;

use crate::input;

/// One line of text, safe to print: every character free text may not
/// contain (controls, bidi and other invisible ones, newline and tab) is
/// written as `\u{…}`. Escaping it again changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PrintableText(String);

impl PrintableText {
    /// `text`, escaped.
    pub(super) fn new(text: &str) -> Self {
        Self(escape_unprintable(text, |_| false))
    }

    #[cfg(test)]
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrintableText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Text that may span lines, safe to print: like [`PrintableText`], but
/// line feeds and tabs are kept. For CSV cells.
#[cfg(feature = "client")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PrintableLines(String);

#[cfg(feature = "client")]
impl PrintableLines {
    /// `text`, escaped but for its line feeds and tabs.
    pub(super) fn new(text: &str) -> Self {
        Self(escape_unprintable(text, |c| c == '\n' || c == '\t'))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A line is lines too: it holds no line feed or tab at all.
#[cfg(feature = "client")]
impl From<PrintableText> for PrintableLines {
    fn from(text: PrintableText) -> Self {
        Self(text.0)
    }
}

/// JSON text, safe to print: serde_json escapes only ASCII controls in
/// strings, so every other character free text may not contain (bidi,
/// zero-width, C1 controls, …) is written as a `\uXXXX` escape too
/// (UTF-16, so a surrogate pair above U+FFFF). The same JSON value.
pub(super) struct PrintableJson(String);

impl PrintableJson {
    /// `value` pretty-printed, escaped.
    #[cfg(feature = "client")]
    pub(super) fn pretty(value: &impl serde::Serialize) -> Self {
        Self(escape_json(
            &serde_json::to_string_pretty(value).expect("JSON values serialize"),
        ))
    }

    /// `value` on one line, escaped.
    #[cfg(feature = "server")]
    pub(super) fn compact(value: &impl serde::Serialize) -> Self {
        Self(escape_json(
            &serde_json::to_string(value).expect("JSON values serialize"),
        ))
    }
}

impl fmt::Display for PrintableJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `text` with every unprintable character but those `keep` accepts
/// written as `\u{…}`.
fn escape_unprintable(text: &str, keep: impl Fn(char) -> bool) -> String {
    text.chars()
        .map(|c| {
            if input::is_unprintable(c) && !keep(c) {
                format!("\\u{{{:x}}}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// JSON text (as serde_json writes it) in which every character free text
/// may not contain is a `\uXXXX` escape.
fn escape_json(json: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_text_escapes_controls_bidi_newlines_and_tabs_once() {
        let text = PrintableText::new("a\u{1b}[31m\u{202e}\n\tb\u{e0041}");

        assert_eq!(
            text.as_str(),
            "a\\u{1b}[31m\\u{202e}\\u{a}\\u{9}b\\u{e0041}"
        );
        assert_eq!(PrintableText::new(text.as_str()), text);
    }

    #[cfg(feature = "client")]
    #[test]
    fn printable_lines_keep_line_feeds_and_tabs() {
        let text = PrintableLines::new("a\r\n\tb\u{7}");

        assert_eq!(text.as_str(), "a\\u{d}\n\tb\\u{7}");
    }
}
