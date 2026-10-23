//! The voting page's JavaScript enhancement: a few small self-hosted ES
//! modules and their stylesheet, embedded in the binary and served under
//! fixed asset routes, and included on every voting page by the app (see
//! [`include`]). The modules are served under one content-versioned
//! directory (`/assets/app/<version>/main.js` and its siblings, see
//! [`MODULES_VERSION`]), so their relative imports carry the version too;
//! the stylesheet is referenced as `?v=<version>`. Browsers may cache
//! either for good.

use std::fmt::Write as _;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::api::rfc3339;
use crate::domain::ballot::{Ballot, ResponseOrigin};

/// The directory the modules are served under, followed by their version
/// and their file name: `/assets/app/<version>/<file>`.
pub(super) const MODULES_DIR: &str = "/assets/app";
/// The module the page loads; it imports the others.
const ENTRY_MODULE: &str = "main.js";
/// Where the script's stylesheet is served.
pub(super) const STYLESHEET_PATH: &str = "/assets/app/ballot.css";

/// The script's modules, by file name: vanilla JavaScript, no build step,
/// no third-party code. Each imports its siblings relatively (`./dom.js`).
pub(super) const MODULES: [(&str, &str); 8] = [
    ("main.js", include_str!("../../static/js/main.js")),
    ("config.js", include_str!("../../static/js/config.js")),
    ("dom.js", include_str!("../../static/js/dom.js")),
    ("time.js", include_str!("../../static/js/time.js")),
    ("api.js", include_str!("../../static/js/api.js")),
    ("dialog.js", include_str!("../../static/js/dialog.js")),
    ("autosave.js", include_str!("../../static/js/autosave.js")),
    ("refresh.js", include_str!("../../static/js/refresh.js")),
];
/// Styles for what the script adds to the page, on the Themes' custom
/// properties.
pub(super) const STYLESHEET: &str = include_str!("../../static/ballot.css");

/// The modules' version: a hash over all of them together (sorted by name,
/// each as its name, a NUL, its content, a NUL), so the pages can load them
/// from `/assets/app/<version>/`, which browsers may cache for good.
pub(super) static MODULES_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut modules = MODULES;
    modules.sort_unstable();
    let mut hasher = Sha256::new();
    for (name, content) in modules {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(content.as_bytes());
        hasher.update([0]);
    }
    hex16(&hasher.finalize())
});
/// The stylesheet's version: the start of the SHA-256 of its content.
pub(super) static STYLESHEET_VERSION: LazyLock<String> =
    LazyLock::new(|| hex16(&Sha256::digest(STYLESHEET.as_bytes())));

/// The module of this file name, if there is one.
pub(super) fn module(name: &str) -> Option<&'static str> {
    MODULES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, content)| *content)
}

/// The first 16 hex digits (64 bits) of a digest.
fn hex16(digest: &[u8]) -> String {
    digest[..8]
        .iter()
        .fold(String::with_capacity(16), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// `html` (a rendered voting page) with the script and its stylesheet
/// included: inserted by the app rather than written in a template, so a
/// Theme overriding `base.html` or `ballot.html` can't drop them. They go
/// right before the first `</head>`; without one, before the first
/// `</body>`; without either, at the end.
///
/// Every URL starts with `base_path` (`/`, or e.g. `/polls/`). The script
/// is a module (`<script type="module">`, deferred by default) with the id
/// `vota-enhancement`, by which it finds its own tag (modules have no
/// `document.currentScript`). What it needs is in `data-*` attributes on
/// that tag (never inline script, which the Content Security Policy
/// forbids):
/// - `data-api`: where to `PUT` the Response (the voter API);
/// - `data-page`: the voting page's own URL;
/// - `data-deadline`: the Voting Deadline, and `data-now`: the server's
///   "now" when rendering (both RFC 3339, UTC), so the countdown can
///   correct for a skewed client clock;
/// - `data-open`: whether voting is still open (`true`/`false`);
/// - `data-revision`: the Ballot's revision as rendered, sent back with the
///   Response;
/// - `data-choice-mode` (`single`/`multiple`), `data-submission-mode`
///   (`final`/`editable`), `data-needs-confirmation` (`true`/`false`);
/// - `data-allow-no-option` (`true`/`false`): whether a Response may select
///   no Option;
/// - `data-answered` (`true`/`false`): whether the Ballot has a Response,
///   so an empty selection on the page tells "no Option" from "no answer";
/// - `data-preselected` (`true`/`false`): whether that Response was
///   recorded from the Preselection and the voter hasn't saved one yet, so
///   the page offers to save it as it stands.
///
/// The script itself relies on nothing else in the page but the no-JS form
/// (a `method="post"` form with `option` inputs and their `<label for>`s),
/// which every Theme's `ballot.html` must keep anyway, and fills a countdown
/// placeholder (`class="vota-countdown"`) if the template has one.
pub(super) fn include(
    html: String,
    ballot: &Ballot,
    now: DateTime<Utc>,
    base_path: &str,
) -> String {
    let token = ballot.token.as_str();
    let tags = format!(
        "<link rel=\"stylesheet\" href=\"{root}{STYLESHEET_PATH}?v={stylesheet_version}\">\n\
         <script type=\"module\" src=\"{root}{MODULES_DIR}/{modules_version}/{ENTRY_MODULE}\" \
         id=\"vota-enhancement\" \
         data-api=\"{root}/api/b/{token}/response\" \
         data-page=\"{root}/b/{token}\" \
         data-deadline=\"{deadline}\" \
         data-now=\"{now}\" \
         data-open=\"{open}\" \
         data-revision=\"{revision}\" \
         data-choice-mode=\"{choice_mode}\" \
         data-submission-mode=\"{submission_mode}\" \
         data-needs-confirmation=\"{needs_confirmation}\" \
         data-allow-no-option=\"{allow_no_option}\" \
         data-answered=\"{answered}\" \
         data-preselected=\"{preselected}\"></script>\n",
        // The base path without its trailing slash: `` at the root.
        root = attribute(base_path.trim_end_matches('/')),
        stylesheet_version = STYLESHEET_VERSION.as_str(),
        modules_version = MODULES_VERSION.as_str(),
        token = attribute(token),
        deadline = attribute(&rfc3339(ballot.voting_deadline)),
        now = attribute(&rfc3339(now)),
        open = ballot.is_open(now),
        revision = ballot.revision,
        choice_mode = attribute(ballot.choice_mode.as_str()),
        submission_mode = attribute(ballot.submission_mode.as_str()),
        needs_confirmation = ballot.needs_confirmation(),
        allow_no_option = ballot.allow_no_option,
        answered = ballot.response.is_some(),
        preselected = ballot
            .response
            .as_ref()
            .is_some_and(|response| response.origin == ResponseOrigin::Preselection),
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
