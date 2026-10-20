//! Rendering a Ballot's Markdown description for the voting page.
//!
//! The source is the admin's sanitised text (see [`crate::input`]); it is
//! rendered at display time, so a stricter renderer applies to existing
//! Ballots too. Two layers:
//!
//! 1. `pulldown-cmark` (CommonMark only, no extensions) parses it, and the
//!    event stream is filtered: raw HTML (block and inline) is dropped, not
//!    escaped through; links whose target isn't an absolute `http`, `https` or
//!    `mailto` URL lose their link (their text stays), and link titles are
//!    dropped; images are dropped, keeping their alt text; headings (any
//!    level) become bold paragraphs, so a description can't outrank the
//!    page's own headings; elements nested deeper than [`MAX_DEPTH`] are
//!    left out, keeping their content. No CommonMark extensions are enabled (no tables,
//!    strikethrough, footnotes …); other blocks the allowlist lacks (rules,
//!    code blocks' `pre`) are reduced to their content by layer 2.
//! 2. `ammonia` cleans the resulting HTML against an explicit allowlist
//!    ([`ALLOWED_TAGS`]; the only attribute is `href` on links, limited to the
//!    same schemes, with `rel="noopener noreferrer nofollow"` added), as
//!    defence in depth should the first layer ever let something through.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use ammonia::{Builder, UrlRelative};
use pulldown_cmark::{CowStr, Event, Parser, Tag, TagEnd, html};

/// Elements that may appear in a rendered description.
const ALLOWED_TAGS: [&str; 10] = [
    "p",
    "br",
    "em",
    "strong",
    "a",
    "ul",
    "ol",
    "li",
    "code",
    "blockquote",
];

/// URL schemes a link may use.
const ALLOWED_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

static CLEANER: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let mut cleaner = Builder::default();
    cleaner
        .tags(HashSet::from(ALLOWED_TAGS))
        .generic_attributes(HashSet::new())
        .generic_attribute_prefixes(HashSet::new())
        .tag_attributes(HashMap::from([("a", HashSet::from(["href"]))]))
        .url_schemes(HashSet::from(ALLOWED_SCHEMES))
        .url_relative(UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer nofollow"));
    cleaner
});

/// The most elements open at once in a rendered description. Deeper ones are
/// left out (their content stays), so that pathological nesting (e.g.
/// thousands of `>`) can't make rendering, which happens on every page view,
/// slow.
const MAX_DEPTH: usize = 16;

/// The description's Markdown `source` as sanitised HTML.
pub(crate) fn render(source: &str) -> String {
    // For each open element of the source: whether it is rendered (so its
    // end is rendered too).
    let mut open: Vec<bool> = Vec::new();
    let events = Parser::new(source).flat_map(|event| match event {
        Event::Html(_) | Event::InlineHtml(_) => vec![],
        Event::Start(tag) => {
            let kept = open.iter().filter(|&&kept| kept).count() < MAX_DEPTH;
            let rendered = if kept { start(tag) } else { vec![] };
            open.push(!rendered.is_empty());
            rendered
        }
        Event::End(tag) => match open.pop() {
            Some(true) => end(tag),
            _ => vec![],
        },
        event => vec![event],
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    CLEANER.clean(&out).to_string()
}

/// How the start of an element renders; nothing drops the element but keeps
/// its content.
fn start(tag: Tag<'_>) -> Vec<Event<'_>> {
    match tag {
        // Images: only their alt text (the content) stays.
        Tag::Image { .. } => vec![],
        // Headings of any level: a paragraph in bold.
        Tag::Heading { .. } => vec![Event::Start(Tag::Paragraph), Event::Start(Tag::Strong)],
        Tag::Link {
            link_type,
            dest_url,
            id,
            title: _,
        } => {
            if !is_allowed_link(&dest_url) {
                return vec![];
            }
            vec![Event::Start(Tag::Link {
                link_type,
                dest_url,
                id,
                title: CowStr::Borrowed(""),
            })]
        }
        tag => vec![Event::Start(tag)],
    }
}

/// How the end of an element whose start rendered ([`start`]) renders.
fn end(tag: TagEnd) -> Vec<Event<'static>> {
    match tag {
        TagEnd::Heading(_) => vec![Event::End(TagEnd::Strong), Event::End(TagEnd::Paragraph)],
        tag => vec![Event::End(tag)],
    }
}

/// Whether `target` is an absolute URL with an allowed scheme.
fn is_allowed_link(target: &str) -> bool {
    url::Url::parse(target).is_ok_and(|url| ALLOWED_SCHEMES.contains(&url.scheme()))
}
