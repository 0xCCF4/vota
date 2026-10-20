//! The voting page's JavaScript enhancement: one small self-hosted script
//! and its stylesheet, embedded in the binary and served under fixed asset
//! routes, and included on every voting page by the app (see [`include`]),
//! referenced with their content versions (`?v=…`, see [`SCRIPT_VERSION`])
//! so browsers can cache them for good.

use std::fmt::Write as _;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::api::rfc3339;
use crate::domain::ballot::Ballot;

/// Where the script is served.
pub(super) const SCRIPT_PATH: &str = "/assets/app/ballot.js";
/// Where the script's stylesheet is served.
pub(super) const STYLESHEET_PATH: &str = "/assets/app/ballot.css";

/// The script: vanilla JavaScript, no build step, no third-party code.
pub(super) const SCRIPT: &str = include_str!("../../static/ballot.js");
/// Styles for what the script adds to the page, on the Themes' custom
/// properties.
pub(super) const STYLESHEET: &str = include_str!("../../static/ballot.css");

/// The script's version: a hash of its content, so the pages can reference
/// it at `?v=<version>`, which browsers may cache for good.
pub(super) static SCRIPT_VERSION: LazyLock<String> = LazyLock::new(|| content_version(SCRIPT));
/// The stylesheet's version, like [`SCRIPT_VERSION`].
pub(super) static STYLESHEET_VERSION: LazyLock<String> =
    LazyLock::new(|| content_version(STYLESHEET));

/// The first 16 hex digits (64 bits) of the SHA-256 of `content`.
fn content_version(content: &str) -> String {
    Sha256::digest(content.as_bytes())[..8].iter().fold(
        String::with_capacity(16),
        |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        },
    )
}

/// `html` (a rendered voting page) with the script and its stylesheet
/// included: inserted by the app rather than written in a template, so a
/// Theme overriding `base.html` or `ballot.html` can't drop them. They go
/// right before the first `</head>`; without one, before the first
/// `</body>`; without either, at the end.
///
/// Every URL starts with `base_path` (`/`, or e.g. `/polls/`). What the
/// script needs is in `data-*` attributes on its `<script>` tag (never
/// inline script, which the Content Security Policy forbids):
/// - `data-api`: where to `PUT` the Response (the voter API);
/// - `data-page`: the voting page's own URL;
/// - `data-deadline`: the Voting Deadline, and `data-now`: the server's
///   "now" when rendering (both RFC 3339, UTC), so the countdown can
///   correct for a skewed client clock;
/// - `data-open`: whether voting is still open (`true`/`false`);
/// - `data-revision`: the Ballot's revision as rendered, sent back with the
///   Response;
/// - `data-choice-mode` (`single`/`multiple`), `data-submission-mode`
///   (`final`/`editable`), `data-needs-confirmation` (`true`/`false`).
///
/// The script itself relies on nothing else in the page but the no-JS form
/// (a `method="post"` form with `option` inputs and their `<label for>`s),
/// which every Theme's `ballot.html` must keep anyway.
pub(super) fn include(
    html: String,
    ballot: &Ballot,
    now: DateTime<Utc>,
    base_path: &str,
) -> String {
    let token = ballot.token.as_str();
    let tags = format!(
        "<link rel=\"stylesheet\" href=\"{root}{STYLESHEET_PATH}?v={stylesheet_version}\">\n\
         <script src=\"{root}{SCRIPT_PATH}?v={script_version}\" defer \
         data-api=\"{root}/api/b/{token}/response\" \
         data-page=\"{root}/b/{token}\" \
         data-deadline=\"{deadline}\" \
         data-now=\"{now}\" \
         data-open=\"{open}\" \
         data-revision=\"{revision}\" \
         data-choice-mode=\"{choice_mode}\" \
         data-submission-mode=\"{submission_mode}\" \
         data-needs-confirmation=\"{needs_confirmation}\"></script>\n",
        // The base path without its trailing slash: `` at the root.
        root = attribute(base_path.trim_end_matches('/')),
        stylesheet_version = STYLESHEET_VERSION.as_str(),
        script_version = SCRIPT_VERSION.as_str(),
        token = attribute(token),
        deadline = attribute(&rfc3339(ballot.voting_deadline)),
        now = attribute(&rfc3339(now)),
        open = ballot.is_open(now),
        revision = ballot.revision,
        choice_mode = attribute(ballot.choice_mode.as_str()),
        submission_mode = attribute(ballot.submission_mode.as_str()),
        needs_confirmation = ballot.needs_confirmation(),
    );
    let at = find_ignoring_case(&html, "</head>")
        .or_else(|| find_ignoring_case(&html, "</body>"))
        .unwrap_or(html.len());
    let mut out = String::with_capacity(html.len() + tags.len());
    out.push_str(&html[..at]);
    out.push_str(&tags);
    out.push_str(&html[at..]);
    out
}

/// The byte offset of the first ASCII-case-insensitive match of `needle`
/// (ASCII) in `haystack`.
fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `value` escaped for a double-quoted HTML attribute. (Every value here is
/// already well-formed, from a validated token, enum or timestamp; this is
/// defence in depth.)
pub(super) fn attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
