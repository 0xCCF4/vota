//! The forms' hidden `revision` field: inserted by the app into every
//! rendered `ballot.html` and `confirm.html` page (see [`include`]), the way
//! the voting page's script is (`enhancement`), so a Theme can't forget it
//! and a voter without JavaScript never gets a 422 because of a Theme.
//!
//! The page is scanned once, the way an HTML parser tokenizes it, but only
//! as far as needed to find form start tags and the inputs in them; it is
//! not parsed into a tree, and nothing but the inserted fields changes:
//! - tag and attribute names match in any case; attribute values may be
//!   double-quoted, single-quoted (and contain `>`) or unquoted, in any
//!   order, with whitespace (and newlines) around `=`; the first of a
//!   repeated attribute counts;
//! - a POST form is a `<form>` whose `method` is `post` in any case;
//!   `<form method="get">` and forms without `method` get nothing;
//! - comments (`<!-- … -->`), doctypes and other `<!…>`/`<?…>` markup,
//!   attribute values, and the content of `script`, `style`, `textarea`,
//!   `title`, `xmp`, `iframe`, `noembed` and `noframes` (text up to their
//!   end tag, however it looks), and everything after `plaintext`, are
//!   skipped, so text that only looks like a form tag is never touched;
//! - a `<form>` start tag inside an open form is ignored and the first
//!   `</form>` closes it, as in a browser; a form left open runs to the end
//!   of the page;
//! - a form that already has an `<input>` named exactly `revision` (the
//!   templates of older Themes write one) is left as it is;
//! - a `select`, `textarea` or `button` named exactly `revision` in a POST
//!   form is reported ([`revision_control`]): the form would post two
//!   `revision` values, so the trial render refuses such a Theme at startup.
//!
//! Limits, none of which the built-in templates or the trial render's
//! samples run into: character references in attribute values are not
//! decoded (`method="&#112;ost"` is not seen as POST); `noscript` content is
//! read as markup (as a browser without JavaScript does); a `<form>` inside
//! `<template>` or SVG/MathML is treated like any other; an input outside
//! the form that names it with `form="…"`, or one that is `disabled`, still
//! counts as the form's field. Every POST form gets the field, whatever it
//! posts to: on these pages each one submits a Response.

/// `html` with `<input type="hidden" name="revision" value="{revision}">`
/// right after the start tag of every `<form method="post">` that doesn't
/// have an input named `revision` yet.
pub(super) fn include(html: String, revision: i64) -> String {
    let field = format!(
        "<input type=\"hidden\" name=\"revision\" value=\"{}\">",
        super::enhancement::attribute(&revision.to_string())
    );
    let mut out = String::with_capacity(html.len() + field.len());
    let mut copied = 0;
    for form in post_forms(&html) {
        if form.has_field {
            continue;
        }
        out.push_str(&html[copied..form.content_start]);
        out.push_str(&field);
        copied = form.content_start;
    }
    out.push_str(&html[copied..]);
    out
}

/// Whether `html` has a `<form method="post">` that [`include`] puts (or
/// finds) the field in.
pub(super) fn has_post_form(html: &str) -> bool {
    !post_forms(html).is_empty()
}

/// The first `select`, `textarea` or `button` named `revision` in a `<form
/// method="post">` of `html`, by its element name (lower case): the field
/// [`include`] adds would be a second `revision` value of that form.
pub(super) fn revision_control(html: &str) -> Option<&'static str> {
    post_forms(html)
        .into_iter()
        .find_map(|form| form.revision_control)
}

/// A `<form method="post">` in a page.
struct PostForm {
    /// Where its content starts: the byte offset right after its start
    /// tag's `>`.
    content_start: usize,
    /// Whether it has an input named `revision` already.
    has_field: bool,
    /// The first other control in it named `revision`, if any.
    revision_control: Option<&'static str>,
}

/// The form controls other than `input` that post a value under their name.
const OTHER_CONTROLS: [&str; 3] = ["select", "textarea", "button"];

/// The page's `<form method="post">`s, in order.
fn post_forms(html: &str) -> Vec<PostForm> {
    let bytes = html.as_bytes();
    let mut forms: Vec<PostForm> = Vec::new();
    // The form open at this point, if any: `Some(Some(index))` for a POST
    // form (its index in `forms`), `Some(None)` for any other form.
    let mut open: Option<Option<usize>> = None;
    let mut i = 0;
    while let Some(lt) = bytes[i..].iter().position(|&b| b == b'<') {
        let start = i + lt;
        let rest = &bytes[start..];
        if rest.starts_with(b"<!--") {
            // A comment; `<!-->` and `<!--->` are empty ones.
            i = match &rest[4..] {
                [b'>', ..] => start + 5,
                [b'-', b'>', ..] => start + 6,
                _ => find(bytes, start + 4, b"-->").map_or(bytes.len(), |at| at + 3),
            };
            continue;
        }
        if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
            // A doctype, or what a browser reads as a comment up to the
            // next `>`.
            i = find(bytes, start, b">").map_or(bytes.len(), |at| at + 1);
            continue;
        }
        let end_tag = bytes.get(start + 1) == Some(&b'/');
        let Some(tag) = Tag::parse(bytes, if end_tag { start + 1 } else { start }) else {
            i = start + 1;
            continue;
        };
        i = tag.end;
        let is = |name: &[u8]| tag.name.eq_ignore_ascii_case(name);
        if !end_tag
            && let Some(Some(form)) = open
            && tag.attribute(b"name") == Some(b"revision")
            && let Some(&control) = OTHER_CONTROLS.iter().find(|name| is(name.as_bytes()))
        {
            forms[form].revision_control.get_or_insert(control);
        }
        if end_tag {
            if is(b"form") {
                open = None;
            }
        } else if let Some(&raw) = RAW_TEXT.iter().find(|name| is(name)) {
            // Its content is text up to its end tag, however it looks.
            i = raw_text_end(bytes, tag.end, raw);
        } else if is(b"plaintext") {
            // Everything after it is text.
            break;
        } else if is(b"form") {
            // A form start tag inside a form is ignored, as by a browser.
            if open.is_none() {
                let post = tag
                    .attribute(b"method")
                    .is_some_and(|method| method.eq_ignore_ascii_case(b"post"));
                open = Some(post.then(|| {
                    forms.push(PostForm {
                        content_start: tag.end,
                        has_field: false,
                        revision_control: None,
                    });
                    forms.len() - 1
                }));
            }
        } else if is(b"input")
            && tag.attribute(b"name") == Some(b"revision")
            && let Some(Some(form)) = open
        {
            forms[form].has_field = true;
        }
    }
    forms
}

/// The elements whose content an HTML parser reads as text up to their end
/// tag (raw text and escapable raw text). `noscript` isn't one of them here:
/// without JavaScript, which is when the forms matter, its content is
/// markup.
const RAW_TEXT: [&[u8]; 8] = [
    b"script",
    b"style",
    b"textarea",
    b"title",
    b"xmp",
    b"iframe",
    b"noembed",
    b"noframes",
];

/// Where the raw text content of the element `name`, starting at `from`,
/// ends: at its end tag (`</name` followed by whitespace, `/` or `>`, any
/// case), or at the end of the page.
fn raw_text_end(bytes: &[u8], from: usize, name: &[u8]) -> usize {
    let mut i = from;
    while let Some(at) = find(bytes, i, b"</") {
        let after = at + 2 + name.len();
        if bytes
            .get(at + 2..after)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            && bytes.get(after).is_none_or(|&b| ends_name(b))
        {
            return at;
        }
        i = at + 2;
    }
    bytes.len()
}

/// The offset of the first `needle` in `bytes` at or after `from`.
fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

/// A start tag: its name, its attributes (names as written, values without
/// their quotes) and the offset right after its `>`.
struct Tag<'a> {
    name: &'a [u8],
    attributes: Vec<(&'a [u8], &'a [u8])>,
    end: usize,
}

impl<'a> Tag<'a> {
    /// The start tag at `start` (a `<`), as an HTML parser reads it: the
    /// name runs to whitespace, `/` or `>`; attribute values may be
    /// double-quoted, single-quoted (either may contain `>`) or unquoted.
    /// `None` if `start` doesn't begin a start tag, or it never ends.
    fn parse(bytes: &'a [u8], start: usize) -> Option<Self> {
        let mut i = start + 1;
        if !bytes.get(i)?.is_ascii_alphabetic() {
            return None;
        }
        let name_start = i;
        while i < bytes.len() && !ends_name(bytes[i]) {
            i += 1;
        }
        let name = &bytes[name_start..i];
        let mut attributes = Vec::new();
        loop {
            while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
                i += 1;
            }
            match bytes.get(i)? {
                b'>' => {
                    return Some(Self {
                        name,
                        attributes,
                        end: i + 1,
                    });
                }
                _ => {
                    let attr_start = i;
                    // An attribute name may start with `=`.
                    i += 1;
                    while i < bytes.len() && !ends_name(bytes[i]) && bytes[i] != b'=' {
                        i += 1;
                    }
                    let attr_name = &bytes[attr_start..i];
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    let mut value: &[u8] = b"";
                    if bytes.get(i) == Some(&b'=') {
                        i += 1;
                        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        match bytes.get(i)? {
                            quote @ (b'"' | b'\'') => {
                                let quote = *quote;
                                let value_start = i + 1;
                                let len = bytes[value_start..].iter().position(|&b| b == quote)?;
                                value = &bytes[value_start..value_start + len];
                                i = value_start + len + 1;
                            }
                            _ => {
                                let value_start = i;
                                while i < bytes.len()
                                    && !bytes[i].is_ascii_whitespace()
                                    && bytes[i] != b'>'
                                {
                                    i += 1;
                                }
                                value = &bytes[value_start..i];
                            }
                        }
                    }
                    attributes.push((attr_name, value));
                }
            }
        }
    }

    /// The value of the attribute `name` (ASCII case-insensitive); the
    /// first one, as for an HTML parser, if it is repeated.
    fn attribute(&self, name: &[u8]) -> Option<&'a [u8]> {
        self.attributes
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    }
}

/// Whether `b` ends a tag or attribute name.
fn ends_name(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'/' || b == b'>'
}
