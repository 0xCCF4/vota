//! The page templates and how a Theme's set of them is assembled.
//!
//! The built-in templates are embedded in the binary. A folder Theme may
//! override any of them ([`TEMPLATE_NAMES`]); the rest come from the
//! built-ins, so e.g. a Theme overriding only `base.html` gets the built-in
//! `ballot.html` extending its own `base.html`. HTML templates are
//! auto-escaped.
//!
//! # Template variables
//!
//! Every page template (`base.html`, `ballot.html`, `confirm.html`) gets:
//! - `theme`: the name of the Theme in use;
//! - `base_path`: the path of `public_base_url`, which every URL of the
//!   app starts with: `/` at the root of a host, or e.g. `/polls/` under a
//!   prefix (always with a leading and a trailing slash, and only ASCII
//!   letters, digits, `-`, `.`, `_`, `~` and `/`, so it needs no escaping).
//!   Link anything of the app as `{{ base_path }}…`, never as `/…`;
//! - `theme_stylesheet`: the root-relative URL of its `style.css`
//!   (`<base_path>assets/themes/<theme>/style.css`);
//! - `accent_color`: `#rrggbb` (lowercase), or none when the Ballot has no
//!   accent colour (the Theme's own then applies);
//! - `accent_contrast`: `#000000` or `#ffffff`, whichever reads better on
//!   the accent colour (none without one);
//! - `logo_url`: the root-relative URL of the Ballot's logo
//!   (`<base_path>assets/logos/<name>`), or none;
//! - `csp_nonce`: the nonce this page's Content Security Policy allows
//!   inline `<style nonce="…">` elements by (inline scripts, `style`
//!   attributes and anything from another host are blocked);
//! - `title`: the Ballot's title; `description`: its description as
//!   sanitised HTML (inserted unescaped), or none.
//!
//! `ballot.html` (the voting page, in every state) also gets:
//! - `options`: the Options, each with `id`, `label` and `checked` (in the
//!   current Response; without one, while voting is open, in the
//!   Preselection);
//! - `multiple` (multiple choice), `editable` (Editable Submission Mode),
//!   `answered` (has a Response), `read_only` (the Response is shown without
//!   a form), `closed` (the Voting Deadline has passed);
//! - `preselected`: whether the ticked Options are the admin's Preselection,
//!   not the voter's choice (a Final Ballot without a Response while open,
//!   or an Editable one whose Response is still the Preselection);
//! - `selected`: the Options of the current Response (`id`, `label`);
//! - `upcoming_fallback`: while voting is open and there is no Response, the
//!   Fallback's Options, or none; `fallback`: once voting has closed, the
//!   applied Fallback's Options, or none;
//! - `message`: a notice for the voter (why a submission was refused or
//!   invalid), or none;
//! - `deadline_utc` (RFC 3339) and `deadline_local` (formatted in the
//!   configured time zone, short: `Tue 1 Dec 2026, 13:00 CET`); `status`:
//!   the Ballot Status as words;
//! - `revision`: the Ballot's revision (an integer). Templates don't need
//!   it: the app inserts the hidden field itself (see below).
//!
//! `confirm.html` (the Final confirmation step) also gets `selected`: the
//! chosen Options, to post again with `confirm=yes`; and `revision`, as
//! above.
//!
//! `base.html` must link `theme_stylesheet` and, to apply the accent colour,
//! set `--accent` in a nonce'd style element (see the built-in one).
//! The forms post to the page itself (`action=""`) and the confirmation
//! page's "go back" link is `href=""`, so they need no base path.
//!
//! Undefined variables are lenient (minijinja's default): they render as
//! nothing and are falsy, so a Theme may test for variables it doesn't know.
//! Every Theme's templates are trial-rendered at startup with sample Ballots
//! in every voting page state, at the base path `/` and under a sample
//! prefix (see `trial_render`); one that fails to render
//! (e.g. `extends` a missing template, or uses an unknown filter), or whose
//! voting page (while it takes a Response) or confirmation step has no
//! `<form method="post">`, stops startup with an error naming the Theme and
//! the template file.
//!
//! # The hidden `revision` field
//!
//! A Response is recorded only against the Ballot as its page showed it, so
//! every no-JS post carries the Ballot's `revision`. Templates don't write
//! that field: after rendering `ballot.html` and `confirm.html`, the app
//! puts `<input type="hidden" name="revision" value="…">` (the current
//! revision) right after the start tag of every `<form method="post">`
//! that doesn't have an `<input name="revision">` yet, so a form gets
//! exactly one and Themes that still write it themselves keep working. The
//! page is scanned like an HTML tokenizer reads it: any case, quoting,
//! attribute order and whitespace, several forms; never inside comments,
//! attribute values, or the text of `script`, `style`, `textarea`,
//! `title` and the like. The details and limits (e.g. character references
//! in `method` are not decoded) are in `revision_field`.
//!
//! # The voting page's script
//!
//! Templates don't include the voting page's script: the app inserts it
//! (`<script type="module" id="vota-enhancement">`, with its stylesheet) into
//! every rendered `ballot.html` page itself, right before the first
//! `</head>` (without one, before `</body>`), so a Theme can't drop it. A Theme must not add inline scripts or event-handler
//! attributes (the Content Security Policy blocks them). The script relies
//! only on what the no-JS page needs anyway: in `ballot.html`, the voting
//! form (`method="post"`, with radio or checkbox inputs named `option`, each
//! with a `<label for>`) and the deadline as `<time datetime="{{ deadline_utc
//! }}">`, which it rewrites into the voter's local time zone. It fills a
//! countdown placeholder (an element with the class `vota-countdown`) if
//! the template has one, and marks the save state of an Option's card (the
//! input's closest `.option`, else its parent) on an Editable Ballot, which
//! it autosaves. It saves in place by swapping in the server's fresh
//! `<body>`, so a page's `<body>` should hold everything that changes with
//! the Ballot Status. Its dialog, status line and countdown are styled by
//! `<base_path>assets/app/ballot.css` with the Themes' custom properties
//! (`--accent`, `--surface`, `--text`, `--muted`, `--border`, `--danger`,
//! `--success`, …, each optional; its buttons are the Theme's own
//! `button`s), on zero-specificity selectors (`.vota-dialog`,
//! `.vota-autosave`, `[data-vota-save]`, …) that a Theme's stylesheet can
//! override.
//! The confirmation page (`confirm.html`) gets no script: with JavaScript
//! the confirmation is a dialog on the voting page instead.

use minijinja::Environment;

/// The templates a Theme consists of, and may override.
pub(crate) const TEMPLATE_NAMES: [&str; 3] = ["base.html", "ballot.html", "confirm.html"];

/// Built-in templates, embedded at compile time.
const BUILT_IN: [(&str, &str); 3] = [
    ("base.html", include_str!("../../templates/base.html")),
    ("ballot.html", include_str!("../../templates/ballot.html")),
    ("confirm.html", include_str!("../../templates/confirm.html")),
];

/// One template of a folder Theme, replacing the built-in of that name.
pub(crate) struct Override {
    /// One of [`TEMPLATE_NAMES`].
    pub(crate) name: &'static str,
    pub(crate) source: String,
}

/// The built-in templates with `overrides` replacing some of them. Fails
/// (naming the template) if an override doesn't parse.
pub(crate) fn environment(
    overrides: Vec<Override>,
) -> Result<Environment<'static>, (&'static str, minijinja::Error)> {
    let mut env = Environment::new();
    for (name, source) in BUILT_IN {
        env.add_template(name, source)
            .expect("the built-in templates parse");
    }
    for Override { name, source } in overrides {
        env.add_template_owned(name, source)
            .map_err(|error| (name, error))?;
    }
    Ok(env)
}
