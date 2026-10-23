//! The client's input boundary, its equivalent of the server's `input/`:
//! everything a server sends back is unboxed only here.
//!
//! [`connection`](super::connection) hands over each response as it
//! arrived, an [`UntrustedValue<Arrival>`]: the status, the headers the
//! client reads (`Location`, `Retry-After`, `Content-Type`) and the body,
//! in private fields. [`read`] is the one sanitizer of a response: an error
//! answer becomes a [`Failure`] whose message holds the server's texts
//! escaped; a successful one becomes a [`Body`], still untrusted, which only
//! the views here read: [`Body::ballot`], [`Body::list`],
//! [`Body::created`] and, for `--json`, [`Body::json`].
//!
//! The views are typed: numbers stay numbers, known enums (Ballot Status,
//! Choice Mode, Submission Mode, a result's source, a Response's origin)
//! are parsed, timestamps are parsed as such, the Ballot's URL is parsed with `url` (only
//! `http`/`https`), and every other string is a [`ServerText`], which can
//! only leave escaped: as a [`PrintableText`], a [`PrintableLines`] (CSV)
//! or inside a [`PrintableJson`]. None of these types has a `Debug` or
//! `Display` that could print the raw text.
//!
//! Parsing is tolerant, as the client always was: fields the client
//! doesn't know are ignored (a newer server may add some), a missing or
//! `null` field is shown as `-` (empty in CSV), and a field of an
//! unexpected type or value (an unknown Ballot Status, a number where text
//! belongs, a URL that doesn't parse) is shown as its text, escaped
//! ([`Shown::Other`]). Only where the client must act on a value is an
//! unreadable one an unexpected answer (exit 1): the URL of `--qr`, the
//! current Voting Deadline of a `+` date, a result's source in `tally`.

use std::convert::Infallible;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap, HeaderName};
use serde_json::Value;
use untrusted_value::{SanitizeWith as _, UntrustedValue};
use url::Url;

use super::exit::Exit;
use super::{ErrorBody, Failure, PrintableJson, PrintableLines, PrintableText};

/// A response as it arrived. Its fields are private: only the sanitizers
/// of this module read them.
pub(super) struct Arrival {
    status: StatusCode,
    location: Option<Vec<u8>>,
    retry_after: Option<Vec<u8>>,
    content_type: Option<Vec<u8>>,
    body: Vec<u8>,
}

impl Arrival {
    /// The response's status, the headers the client reads and its body,
    /// wrapped as untrusted.
    pub(super) fn wrap(
        status: StatusCode,
        headers: &HeaderMap,
        body: Vec<u8>,
    ) -> UntrustedValue<Self> {
        let header = |name: HeaderName| headers.get(name).map(|value| value.as_bytes().to_vec());
        UntrustedValue::wrap(Self {
            status,
            location: header(header::LOCATION),
            retry_after: header(header::RETRY_AFTER),
            content_type: header(header::CONTENT_TYPE),
            body,
        })
    }
}

/// The JSON body of a successful answer, still untrusted: only the views
/// of this module read it. The default is JSON `null` (what a `204` shows
/// as).
pub(super) struct Body(UntrustedValue<Value>);

impl Default for Body {
    fn default() -> Self {
        Self(UntrustedValue::wrap(Value::Null))
    }
}

/// A successful answer: its JSON body, `None` for `204 No Content`.
pub(super) type Reply = Option<Body>;

/// Unboxes `value` with a sanitizer that can't fail.
fn unbox<I, T>(value: UntrustedValue<I>, sanitizer: impl FnOnce(I) -> T) -> T {
    let Ok(trusted) = value.sanitize_with(|insecure| Ok::<T, Infallible>(sanitizer(insecure)));
    trusted
}

/// The sanitizer of every response: on success its JSON body (an
/// unexpected answer, exit 1, without one); otherwise a [`Failure`] with
/// the exit code of its status, a message naming the server's error
/// (escaped), and the error body for `--json`.
pub(super) fn read(arrival: UntrustedValue<Arrival>) -> Result<Reply, Failure> {
    arrival.sanitize_with(|arrival| {
        let json: Option<Value> = serde_json::from_slice(&arrival.body).ok();
        let status = arrival.status;
        if status.is_success() {
            if status == StatusCode::NO_CONTENT {
                return Ok(None);
            }
            return match json {
                Some(json) => Ok(Some(Body(UntrustedValue::wrap(json)))),
                None => Err(Failure::new(
                    Exit::Other,
                    format!("unexpected response: {status} without a JSON body"),
                )),
            };
        }
        if status.is_redirection() {
            let location = arrival
                .location
                .as_deref()
                .map_or_else(|| PrintableText::new("nowhere"), location);
            return Err(Failure::new(
                Exit::Other,
                format!(
                    "{status}: redirected to {location}; redirects are not followed (check --url)"
                ),
            ));
        }
        Err(failure(arrival, json))
    })
}

/// A `Location`: the URL it names (in the `url` crate's ASCII form), or
/// its text, escaped, when it is relative or no URL at all.
fn location(value: &[u8]) -> PrintableText {
    let text = String::from_utf8_lossy(value);
    match Url::parse(&text) {
        Ok(url) => PrintableText::new(url.as_str()),
        Err(_) => PrintableText::new(&text),
    }
}

/// The [`Failure`] of an error answer (`json`: its body, if JSON).
fn failure(arrival: Arrival, json: Option<Value>) -> Failure {
    let status = arrival.status;
    let mut message = status.as_u16().to_string();
    match &json {
        Some(error) if error.get("error").is_some_and(Value::is_string) => {
            let text = |key: &str| error.get(key).and_then(Value::as_str);
            message += &format!(" {}", PrintableText::new(text("error").unwrap_or_default()));
            if let Some(detail) = text("message") {
                message += &format!(": {}", PrintableText::new(detail));
            }
            if let Some(field) = text("field").filter(|field| !field.is_empty()) {
                message += &format!(" (field: {})", PrintableText::new(field));
            }
        }
        _ => {
            message += &format!(" {}", status.canonical_reason().unwrap_or("unknown status"));
            // A short plain-text reason (such as the body limit's) is
            // worth showing; an HTML page, say from a proxy, is not.
            let plain = (arrival.content_type.as_deref())
                .and_then(|value| std::str::from_utf8(value).ok())
                .is_some_and(|value| value.starts_with("text/plain"));
            let text = String::from_utf8_lossy(&arrival.body);
            let text = text.trim();
            if plain && !text.is_empty() {
                let shown: String = text.chars().take(200).collect();
                message += &format!(": {}", PrintableText::new(&shown));
            }
        }
    }
    if status == StatusCode::UNAUTHORIZED {
        message += " (the Admin Token was refused: unknown, revoked or expired)";
    }
    if let Some(retry_after) = &arrival.retry_after {
        let value = String::from_utf8_lossy(retry_after);
        message += &match value.trim().parse::<u64>() {
            Ok(seconds) => format!(" (retry after {seconds} seconds)"),
            Err(_) => format!(" (Retry-After: {})", PrintableText::new(&value)),
        };
    }
    // A refused bulk create names the Voter References that are taken.
    let taken: Vec<String> = (json.iter())
        .filter_map(|answer| answer.get("voter_references")?.as_array())
        .flatten()
        .map(|reference| as_text(reference).text().to_string())
        .collect();
    if !taken.is_empty() {
        message += &format!(" (taken: {})", taken.join(", "));
    }
    let body = match json {
        Some(json) => Some(ErrorBody::Json(PrintableJson::pretty(&json))),
        None if arrival.body.is_empty() => None,
        None => Some(ErrorBody::Text(PrintableText::new(
            &String::from_utf8_lossy(&arrival.body),
        ))),
    };
    Failure {
        exit: Exit::for_status(status),
        message: PrintableText::new(&message),
        body,
    }
}

impl Body {
    /// The body as JSON for `--json`: the parsed answer re-serialized
    /// (pretty-printed), every unprintable character in a string escaped.
    pub(super) fn json(&self) -> PrintableJson {
        unbox(self.0.clone(), |value| PrintableJson::pretty(&value))
    }

    /// The admin view of one Ballot.
    pub(super) fn ballot(&self) -> ServerBallot {
        unbox(self.0.clone(), |value| ServerBallot::of(&value))
    }

    /// The Ballots of `GET /admin/ballots` (`{"ballots": [...]}`).
    pub(super) fn list(&self) -> Vec<ServerBallot> {
        unbox(self.0.clone(), |value| ballots(value.get("ballots")))
    }

    /// The answer of `create`: many Ballots (`{"ballots": [...]}`) or one.
    pub(super) fn created(&self) -> Created {
        unbox(self.0.clone(), |value| match value.get("ballots") {
            Some(list) => Created::Many(ballots(Some(list))),
            None => Created::One(Box::new(ServerBallot::of(&value))),
        })
    }
}

/// The Ballots of a JSON array; none for anything else.
fn ballots(list: Option<&Value>) -> Vec<ServerBallot> {
    (list.and_then(Value::as_array).into_iter().flatten())
        .map(ServerBallot::of)
        .collect()
}

/// The answer of `create`.
pub(super) enum Created {
    One(Box<ServerBallot>),
    Many(Vec<ServerBallot>),
}

/// Text from the server, sealed: it leaves only escaped, as
/// [`Self::text`], [`Self::lines`], [`Self::each_line`], or serialized into
/// a [`PrintableJson`]. Compared and ordered by the text itself.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ServerText(String);

impl ServerText {
    /// On one line, escaped.
    pub(super) fn text(&self) -> PrintableText {
        PrintableText::new(&self.0)
    }

    /// Escaped but for line feeds and tabs, for a CSV cell.
    pub(super) fn lines(&self) -> PrintableLines {
        PrintableLines::new(&self.0)
    }

    /// Each of its lines (as [`str::lines`] splits them), escaped.
    pub(super) fn each_line(&self) -> Vec<PrintableText> {
        self.0.lines().map(PrintableText::new).collect()
    }
}

/// For a JSON document the client builds (`tally --json`), which is only
/// ever printed as a [`PrintableJson`].
impl serde::Serialize for ServerText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// A value the client shows: its readable form and its CSV cell.
pub(super) trait Show {
    fn show(&self) -> PrintableText;

    fn cell(&self) -> PrintableLines {
        self.show().into()
    }
}

impl Show for ServerText {
    fn show(&self) -> PrintableText {
        self.text()
    }

    fn cell(&self) -> PrintableLines {
        self.lines()
    }
}

impl Show for u64 {
    fn show(&self) -> PrintableText {
        PrintableText::new(&self.to_string())
    }
}

/// `yes` or `no`.
impl Show for bool {
    fn show(&self) -> PrintableText {
        PrintableText::new(if *self { "yes" } else { "no" })
    }
}

/// In the `url` crate's form, which is ASCII.
impl Show for Url {
    fn show(&self) -> PrintableText {
        PrintableText::new(self.as_str())
    }
}

/// A field of an answer.
#[derive(Clone, Default, PartialEq)]
pub(super) enum Shown<T> {
    /// Absent or `null`.
    #[default]
    Missing,
    /// What the field should hold.
    Known(T),
    /// Something else: a string as it is, any other JSON value as its JSON
    /// text.
    Other(ServerText),
}

impl<T> Shown<T> {
    pub(super) fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Missing | Self::Other(_) => None,
        }
    }
}

impl<T: Show> Shown<T> {
    /// Readable: `-` when missing.
    pub(super) fn show(&self) -> PrintableText {
        match self {
            Self::Missing => PrintableText::new("-"),
            Self::Known(value) => value.show(),
            Self::Other(text) => text.text(),
        }
    }

    /// A CSV cell: empty when missing.
    pub(super) fn cell(&self) -> PrintableLines {
        match self {
            Self::Missing => PrintableLines::default(),
            Self::Known(value) => value.cell(),
            Self::Other(text) => text.lines(),
        }
    }
}

/// The field `key` of `object`, read by `parse`; [`Shown::Other`] when
/// `parse` can't read it.
fn field<T>(object: &Value, key: &str, parse: impl FnOnce(&Value) -> Option<T>) -> Shown<T> {
    match object.get(key) {
        None | Some(Value::Null) => Shown::Missing,
        Some(value) => parse(value).map_or_else(|| Shown::Other(as_text(value)), Shown::Known),
    }
}

/// A string as it is, any other JSON value as its JSON text.
fn as_text(value: &Value) -> ServerText {
    match value {
        Value::String(text) => ServerText(text.clone()),
        other => ServerText(other.to_string()),
    }
}

/// A string.
fn text(value: &Value) -> Option<ServerText> {
    value.as_str().map(|text| ServerText(text.to_owned()))
}

/// An `http` or `https` URL.
fn web_url(value: &Value) -> Option<Url> {
    Url::parse(value.as_str()?)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
}

/// A timestamp from the server: shown as the server wrote it, and the
/// instant it names.
#[derive(Clone, PartialEq)]
pub(super) struct Timestamp {
    text: String,
    instant: DateTime<Utc>,
}

impl Timestamp {
    fn parse(value: &Value) -> Option<Self> {
        let text = value.as_str()?;
        Some(Self {
            instant: text.parse().ok()?,
            text: text.to_owned(),
        })
    }

    pub(super) fn instant(&self) -> DateTime<Utc> {
        self.instant
    }
}

impl Show for Timestamp {
    fn show(&self) -> PrintableText {
        PrintableText::new(&self.text)
    }
}

/// Enums of the admin view: parsed from, and shown as, their API names.
macro_rules! api_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(super) enum $name {
            $($variant),+
        }

        impl $name {
            fn parse(value: &Value) -> Option<Self> {
                match value.as_str()? {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl Show for $name {
            fn show(&self) -> PrintableText {
                PrintableText::new(match self {
                    $(Self::$variant => $text),+
                })
            }
        }
    };
}

api_enum!(
    /// A Ballot Status.
    Status { Unanswered = "unanswered", Answered = "answered", AutoFallback = "auto_fallback" }
);
api_enum!(
    /// A Choice Mode.
    ChoiceMode { Single = "single", Multiple = "multiple" }
);
api_enum!(
    /// A Submission Mode.
    SubmissionMode { Final = "final", Editable = "editable" }
);
api_enum!(
    /// Where a Ballot's result comes from.
    Source { Response = "response", Preselection = "preselection", Fallback = "fallback" }
);
api_enum!(
    /// Where a Response comes from.
    Origin { Voter = "voter", Preselection = "preselection" }
);

/// The admin view of a Ballot, as far as the client shows it. Anything
/// but a JSON object is a Ballot of missing fields.
#[derive(Default)]
pub(super) struct ServerBallot {
    pub(super) id: Shown<u64>,
    pub(super) title: Shown<ServerText>,
    pub(super) url: Shown<Url>,
    pub(super) status: Shown<Status>,
    pub(super) group: Shown<ServerText>,
    pub(super) voter_reference: Shown<ServerText>,
    pub(super) choice_mode: Shown<ChoiceMode>,
    pub(super) allow_no_option: Shown<bool>,
    pub(super) submission_mode: Shown<SubmissionMode>,
    pub(super) voting_deadline: Shown<Timestamp>,
    pub(super) purge_date: Shown<Timestamp>,
    pub(super) created_at: Shown<Timestamp>,
    pub(super) theme: Shown<ServerText>,
    pub(super) accent_color: Shown<ServerText>,
    pub(super) logo: Shown<ServerText>,
    pub(super) description: Shown<ServerText>,
    /// The Options; none when it isn't a list.
    pub(super) options: Vec<ServerOption>,
    /// The Fallback's Options; none when it isn't a list.
    pub(super) fallback: Vec<ServerOption>,
    /// The Preselection's Options; none when it isn't a list.
    pub(super) preselection: Vec<ServerOption>,
    /// The Response, when it is an object.
    pub(super) response: Option<ServerResponse>,
    /// The result, when it is an object.
    pub(super) result: Option<ServerResult>,
}

/// An Option: `{id, label}`.
#[derive(Clone)]
pub(super) struct ServerOption {
    pub(super) id: Shown<ServerText>,
    pub(super) label: Shown<ServerText>,
}

/// A Ballot's Response.
pub(super) struct ServerResponse {
    /// The chosen Options' IDs; none when it isn't a list.
    pub(super) option_ids: Vec<Shown<ServerText>>,
    pub(super) answered_at: Shown<Timestamp>,
    pub(super) changed_at: Shown<Timestamp>,
    /// The voter's, or the Preselection's.
    pub(super) origin: Shown<Origin>,
}

/// A Ballot's result.
pub(super) struct ServerResult {
    pub(super) source: Shown<Source>,
    /// The chosen Options; `None` when it isn't a list (told apart from an
    /// empty list: an answer of no Option).
    pub(super) options: Option<Vec<ServerOption>>,
}

impl ServerBallot {
    fn of(value: &Value) -> Self {
        Self {
            id: field(value, "id", Value::as_u64),
            title: field(value, "title", text),
            url: field(value, "url", web_url),
            status: field(value, "status", Status::parse),
            group: field(value, "group", text),
            voter_reference: field(value, "voter_reference", text),
            choice_mode: field(value, "choice_mode", ChoiceMode::parse),
            allow_no_option: field(value, "allow_no_option", Value::as_bool),
            submission_mode: field(value, "submission_mode", SubmissionMode::parse),
            voting_deadline: field(value, "voting_deadline", Timestamp::parse),
            purge_date: field(value, "purge_date", Timestamp::parse),
            created_at: field(value, "created_at", Timestamp::parse),
            theme: field(value, "theme", text),
            accent_color: field(value, "accent_color", text),
            logo: field(value, "logo", text),
            description: field(value, "description", text),
            options: options(value.get("options")).unwrap_or_default(),
            fallback: options(value.get("fallback")).unwrap_or_default(),
            preselection: options(value.get("preselection")).unwrap_or_default(),
            response: (value
                .get("response")
                .filter(|response| response.is_object()))
            .map(|response| ServerResponse {
                option_ids: (response.get("option_ids").and_then(Value::as_array))
                    .into_iter()
                    .flatten()
                    .map(|id| match id {
                        Value::Null => Shown::Missing,
                        id => text(id).map_or_else(|| Shown::Other(as_text(id)), Shown::Known),
                    })
                    .collect(),
                answered_at: field(response, "answered_at", Timestamp::parse),
                changed_at: field(response, "changed_at", Timestamp::parse),
                origin: field(response, "origin", Origin::parse),
            }),
            result: (value.get("result").filter(|result| result.is_object())).map(|result| {
                ServerResult {
                    source: field(result, "source", Source::parse),
                    options: options(result.get("options")),
                }
            }),
        }
    }
}

/// The Options of a JSON array; `None` for anything else.
fn options(list: Option<&Value>) -> Option<Vec<ServerOption>> {
    let list = list?.as_array()?;
    Some(
        list.iter()
            .map(|option| ServerOption {
                id: field(option, "id", text),
                label: field(option, "label", text),
            })
            .collect(),
    )
}
