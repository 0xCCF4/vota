//! The input boundary: the only place where values from outside are unboxed.
//!
//! Every inbound value (JSON body, form body, path segment, query parameter)
//! enters as an [`UntrustedValue`](untrusted_value::UntrustedValue) through the
//! extractors in [`extract`], and the only way out is a sanitizer in this
//! module. Sanitizers produce validated domain types ([`crate::domain`]), whose
//! constructors demand a [`Sanitized`] proof that only this module can create.
//! Request DTOs keep their fields private, so a handler can pass one on but
//! never read it. Security review can therefore focus here.
//!
//! Rules (spec, "Sanitization at the boundary"):
//! - text: NFC-normalised, trimmed, non-empty, no control, format (bidi,
//!   zero-width, …), private-use or unassigned characters, noncharacters or
//!   other invisible ones (fillers, variation selectors, U+2800), at most
//!   four combining marks per base character, capped in characters after
//!   normalisation ([`text`]); the
//!   Markdown description alone may span lines: `\r\n` and `\r` become `\n`,
//!   and newline and tab are allowed (10 000 characters; stored as sanitised
//!   source, rendered only for display);
//! - Ballot Token: exactly 22 canonical base64url characters, literally in
//!   the raw request path (no percent-encoding), checked before any database
//!   lookup, otherwise the surface's uniform not-found response;
//! - internal Ballot IDs (admin path segment): a positive decimal integer as
//!   the server writes it (digits only, no sign or leading zero), literally
//!   in the raw request path (no percent-encoding), otherwise the admin API's
//!   not-found response;
//! - revision (voter Responses): the revision of the Ballot the voter saw,
//!   required; in JSON an integer from 0 to 2^63-1 (no float, string or
//!   `null`), in the form exactly one value of plain decimal digits (no sign,
//!   leading zero or spaces); otherwise a 422 (`revision`, or the form's
//!   notice). Checked before the selection: one that is not the Ballot's
//!   current revision is [`SelectionRejection::Stale`] / [`FormProblem::Stale`],
//!   answered as "the Ballot changed" (409);
//! - Option IDs: exactly 11 canonical base64url characters, then membership in
//!   the Ballot; a selection has no duplicates and obeys the Ballot's rules
//!   (empty only if the Ballot allows no Option; in the form, no `option`
//!   field at all is the empty selection);
//!   malformed IDs and duplicates are 422s, but a well-formed ID that is not
//!   one of the Ballot's current Options (most likely one the admin has
//!   since replaced) is [`SelectionRejection::NotOnBallot`] /
//!   [`FormProblem::NotOnBallot`], answered as "the Ballot changed" (409),
//!   the same whether or not the ID exists on another Ballot;
//! - Fallback (given by Option labels, as Options get new IDs on creation and
//!   on replacement): each label sanitized like an Option label and matching
//!   one of the Ballot's (resulting) Options exactly, no duplicates, obeying
//!   the Choice Mode (never empty);
//! - Theme and logo names (admin fields `theme`, `logo`; asset path
//!   segments): `^[a-z0-9][a-z0-9-]{0,63}$` by exact match (no trimming or
//!   case folding), then looked up among the Theme registry's known names
//!   only, never joined onto a path; an unknown name is a 422 (`theme` /
//!   `logo`) in the admin API and a plain 404 on the asset routes, where the
//!   segment must also be literal (no percent-encoding);
//! - accent colour (`accent_color`): `^#[0-9a-fA-F]{6}$` exactly, stored
//!   lowercase, and only ever rendered as the value of the CSS custom
//!   property `--accent`;
//! - enums (Choice Mode, Submission Mode): an exact match against the known
//!   names, no trimming or case folding; absent means the default (creation)
//!   or the current value (edit), `null` is rejected;
//! - Allow No Option (`allow_no_option`): a JSON boolean, `null` rejected;
//!   absent means `false` (creation) or the current value (edit); `true`
//!   only with the resulting Choice Mode `multiple` (a kept `true` with a
//!   change to single choice is the `choice_mode`'s fault);
//! - timestamps: RFC 3339 with an explicit offset, or a local date-time
//!   (`YYYY-MM-DDTHH:MM[:SS[.f]]`, exactly) in the configured
//!   `default_time_zone`, refused if a clock change skips or repeats it;
//!   converted to UTC; a Voting
//!   Deadline must lie in the future at creation, on edit and on extension
//!   (where it must also be later than the current one); a Purge Date given at
//!   creation or on edit must lie in the future and not before the
//!   (resulting) Voting Deadline; both, when given, no later than now plus
//!   the configured `max_date_horizon` ([`DateBounds`]);
//! - bulk creation (`POST /admin/ballots`): `voter_reference` may be a
//!   list of 1 to 1000 Voter References, each sanitized like one, without
//!   duplicates after normalisation (`voter_reference[i]`); a longer list is
//!   refused before its entries are sanitized. `count` is an integer from 1
//!   to 1000 and needs a `group` and no `voter_reference`; the rules live in
//!   `domain::bulk`;
//! - admin edits (`PATCH`): any subset of the creation fields plus
//!   `purge_date`; absent keeps the current value; `null` is rejected except
//!   for `description`, `fallback`, `group`, `voter_reference`,
//!   `accent_color` and `logo`, where it removes the value (so `"theme":
//!   null` is a 422; name `classic` to return to the default Theme); a
//!   Theme or logo the edit leaves unchanged is not checked again (it may
//!   have been removed from its folder since); the rules apply to the Ballot as the edit leaves it (e.g. the
//!   current Fallback is re-resolved by label against replaced Options and
//!   checked against a changed Choice Mode); an edit naming only `group`
//!   and/or `voter_reference` is bookkeeping, allowed on an Answered or
//!   Auto-fallback Ballot too, any other field makes it a content edit
//!   (presence-based);
//! - query strings (admin list): parsed like forms, unknown parameters
//!   rejected, each parameter at most once; a Group filter is sanitized like a
//!   Group;
//! - forms (no-JS voting page): URL-encoded only, unknown fields rejected,
//!   every field read as a list of values; the same Option rules as JSON;
//! - app asset query (`/assets/app/ballot.css?v=<version>`): exactly `v=`
//!   and the current version, compared as is (no decoding), or it is not
//!   the current version; never rejected, only cached briefly;
//! - app module path (`/assets/app/{version}/{file}`): both segments
//!   literal (any percent-encoding is a plain 404); the version is the
//!   current one by exact match or else not (never rejected, only cached
//!   briefly); the file is one of the embedded modules' names by exact
//!   match, or a plain 404;
//! - `X-Forwarded-For`: read only from a trusted proxy (ignored from any other
//!   peer), a strict list of at most 32 bare IP addresses; anything else from a
//!   trusted proxy is a 400 on the rate-limited voter routes, charged to no
//!   rate-limit bucket ([`client_ip`]);
//! - Admin Token labels (`admin-token create --label`, not HTTP): like any
//!   single-line text, at most 200 characters ([`admin_token`]);
//! - Admin Token files (`admin_token_files`, read at startup, not HTTP):
//!   UTF-8 holding exactly one Admin Token, `vta_` and 43 canonical
//!   base64url characters, after trimming surrounding whitespace; kept as
//!   its SHA-256 hash only. The error never quotes the contents
//!   ([`admin_token_file`]);
//! - JSON: an object at the top level, unknown fields rejected, trailing data
//!   rejected; body size limited to [`BODY_LIMIT_BYTES`] on every route (413
//!   above it).
//!
//! Sanitization errors become 422 responses naming the offending field
//! ([`InputError`]), if the schema knows it: a field or parameter it doesn't
//! know (or a name that isn't UTF-8) is reported as `null`, never echoed;
//! a bad Ballot Token is the uniform 404 instead. The no-JS
//! form gets a [`FormProblem`] instead, which the voting page names in words
//! (never echoing the submitted value).

// The server's request types and sanitizers (cargo feature `server`); the
// character rules are shared with the CLI client.
#[cfg(feature = "server")]
mod admin_token;
#[cfg(feature = "server")]
mod ballot;
#[cfg(feature = "server")]
mod ballot_id;
#[cfg(feature = "server")]
mod ballot_token;
#[cfg(feature = "server")]
mod base64url;
mod characters;
#[cfg(feature = "server")]
mod client_ip;
#[cfg(feature = "server")]
mod error;
#[cfg(feature = "server")]
mod extract;
#[cfg(feature = "server")]
mod response;
#[cfg(feature = "server")]
mod text;
#[cfg(feature = "server")]
mod theme;

#[cfg(feature = "server")]
pub use admin_token::admin_token_file;
#[cfg(feature = "server")]
pub use ballot::{
    CreateBallotRequest, DateBounds, EditBallotRequest, ExtendRequest, ListBallotsQuery,
    ballot_changes, ballot_filter, edit_scope, extension, new_ballot,
};
pub use characters::is_unprintable;
#[cfg(feature = "server")]
pub use client_ip::{client_ip, is_local_unproxied};
#[cfg(feature = "server")]
pub use error::InputError;
#[cfg(feature = "server")]
pub use extract::{
    AppModulePath, AssetNamePath, AssetQuery, BallotIdPath, BallotTokenPath, BodyRejection,
    NotFound, UntrustedForm, UntrustedJson, UntrustedQuery,
};
#[cfg(feature = "server")]
pub use response::{
    FormProblem, ResponseForm, ResponseRequest, SelectionRejection, api_response, form_response,
};
#[cfg(feature = "server")]
pub use theme::{
    app_module, is_current_version, is_current_version_segment, logo_asset, theme_asset,
};

/// Maximum request body size on every route.
#[cfg(feature = "server")]
pub const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Proof that a value went through a sanitizer. Only this module can create it,
/// so validated domain types can't be constructed from raw input elsewhere.
#[cfg(feature = "server")]
#[derive(Debug, Clone, Copy)]
pub struct Sanitized(());

#[cfg(feature = "server")]
impl Sanitized {
    const PROOF: Self = Self(());
}
