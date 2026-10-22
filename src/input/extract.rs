//! Extractors: raw HTTP input -> `UntrustedValue`.

use std::marker::PhantomData;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, OriginalUri, Path, Request};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::json;
use untrusted_value::{SanitizeValue as _, UntrustedValue};

use super::InputError;
use super::text::Rejection;
use crate::domain::ballot::{BallotId, BallotToken};

/// A JSON body, deserialized (must be an object; unknown fields rejected) but
/// still untrusted.
pub struct UntrustedJson<T>(pub UntrustedValue<T>);

/// A URL-encoded form body (what a browser posts without JavaScript),
/// deserialized (unknown fields rejected) but still untrusted.
///
/// A form field may appear any number of times, so every field of `T` must be
/// a `Vec<String>` (with `#[serde(default)]` when it may be absent). Invalid
/// UTF-8 in names or values is rejected.
pub struct UntrustedForm<T>(pub UntrustedValue<T>);

/// Rejection of [`UntrustedJson`] and [`UntrustedForm`].
pub enum BodyRejection {
    UnsupportedMediaType,
    /// Body too large or unreadable; carries axum's own response (e.g. 413).
    Body(Response),
    /// The body is not syntactically valid JSON (truncated, empty, not
    /// UTF-8, trailing data).
    NotJson,
    Invalid(InputError),
}

impl IntoResponse for BodyRejection {
    fn into_response(self) -> Response {
        match self {
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
            Self::Body(response) => response,
            Self::NotJson => {
                let body = json!({
                    "error": "invalid_json",
                    "message": "the request body is not valid JSON",
                });
                (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
            }
            Self::Invalid(error) => error.into_response(),
        }
    }
}

impl<S, T> FromRequest<S> for UntrustedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = BodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !has_content_type(&request, "application/json") {
            return Err(BodyRejection::UnsupportedMediaType);
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| BodyRejection::Body(rejection.into_response()))?;
        // A body that is not JSON at all gets one fixed answer: serde's
        // position (and any path) in a broken document says nothing useful.
        // It is parsed in full first, so that a data error early in the body
        // (e.g. an unknown field) can't hide a broken end. Building values
        // also checks what skipping through would not: numbers out of range,
        // lone surrogate escapes. JSON text is UTF-8 as a whole (RFC 8259).
        if std::str::from_utf8(&bytes).is_err()
            || serde_json::from_slice::<serde_json::Value>(&bytes).is_err()
        {
            return Err(BodyRejection::NotJson);
        }
        // Every JSON body is an object. (serde would also build a struct from
        // an array, so e.g. `[]` would pass as an edit with no fields.)
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
            return Err(BodyRejection::Invalid(InputError::new(
                "",
                "must be a JSON object",
            )));
        }
        let deserializer = &mut serde_json::Deserializer::from_slice(&bytes);
        let value: T = serde_path_to_error::deserialize(&mut *deserializer).map_err(|error| {
            if error.inner().is_syntax() || error.inner().is_eof() {
                return BodyRejection::NotJson;
            }
            let path = error.path().to_string();
            let path = if path == "." { "" } else { path.as_str() };
            BodyRejection::Invalid(json_error(path, error.inner()))
        })?;
        // Trailing data after the JSON value (already refused above).
        deserializer.end().map_err(|_| BodyRejection::NotJson)?;
        Ok(Self(UntrustedValue::wrap(value)))
    }
}

impl<S, T> FromRequest<S> for UntrustedForm<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = BodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !has_content_type(&request, "application/x-www-form-urlencoded") {
            return Err(BodyRejection::UnsupportedMediaType);
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| BodyRejection::Body(rejection.into_response()))?;
        let value: T = from_urlencoded(&bytes).map_err(BodyRejection::Invalid)?;
        Ok(Self(UntrustedValue::wrap(value)))
    }
}

/// The URL query string, deserialized (unknown parameters rejected) but still
/// untrusted. Parsed like [`UntrustedForm`]: every field of `T` must be a
/// `Vec<String>` (with `#[serde(default)]` when it may be absent). A missing
/// query is an empty one.
pub struct UntrustedQuery<T>(pub UntrustedValue<T>);

impl<S, T> FromRequestParts<S> for UntrustedQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = InputError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        let value: T = from_urlencoded(query.as_bytes())?;
        Ok(Self(UntrustedValue::wrap(value)))
    }
}

/// Deserializes URL-encoded `name=value` pairs into `T`, each field as the
/// list of its values (in order). Invalid UTF-8 after percent-decoding is
/// rejected: in a value, naming its field if `T` has it (otherwise it is an
/// unknown field); in a name, which then can't be one of `T`'s, unnamed.
fn from_urlencoded<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InputError> {
    let mut fields = serde_json::Map::new();
    // The first name whose value is not UTF-8.
    let mut undecodable = None;
    for pair in bytes.split(|&b| b == b'&').filter(|pair| !pair.is_empty()) {
        let mut parts = pair.splitn(2, |&b| b == b'=');
        let name = decode_component(parts.next().unwrap_or_default())
            .ok_or_else(|| InputError::unnamed("invalid UTF-8"))?;
        let value = decode_component(parts.next().unwrap_or_default()).unwrap_or_else(|| {
            undecodable.get_or_insert_with(|| name.clone());
            String::new()
        });
        let values = fields
            .entry(name)
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let serde_json::Value::Array(values) = values {
            values.push(serde_json::Value::String(value));
        }
    }
    let result = deserialize_fields::<T>(fields);
    match (result, undecodable) {
        (Err(error), _) if error.is_unknown_field() => Err(error),
        (_, Some(name)) if is_field_of::<T>(&name) => Err(InputError::new(name, "invalid UTF-8")),
        (_, Some(_)) => Err(InputError::unknown_field()),
        (result, None) => result,
    }
}

fn deserialize_fields<T: DeserializeOwned>(
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<T, InputError> {
    serde_path_to_error::deserialize(serde_json::Value::Object(fields)).map_err(|error| {
        let path = error.path().to_string();
        let path = if path == "." { "" } else { path.as_str() };
        json_error(path, error.inner())
    })
}

/// Whether `T` (a struct of form fields that refuses unknown ones) has a
/// field called `name`: one given alone is either refused as unknown or not.
fn is_field_of<T: DeserializeOwned>(name: &str) -> bool {
    let alone = serde_json::Map::from_iter([(name.to_owned(), serde_json::json!([""]))]);
    !matches!(deserialize_fields::<T>(alone), Err(error) if error.is_unknown_field())
}

/// One `application/x-www-form-urlencoded` name or value: `+` is a space,
/// then percent-decoding; `None` unless the result is valid UTF-8.
fn decode_component(raw: &[u8]) -> Option<String> {
    let plus_as_space: Vec<u8> = raw
        .iter()
        .map(|&b| if b == b'+' { b' ' } else { b })
        .collect();
    let decoded: Vec<u8> = percent_encoding::percent_decode(&plus_as_space).collect();
    String::from_utf8(decoded).ok()
}

/// Whether the request declares this media type (parameters ignored).
fn has_content_type(request: &Request, mime: &str) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|declared| declared.trim().eq_ignore_ascii_case(mime))
}

/// Maps a deserialization error at `path` to a 422. serde's messages may echo
/// input, so only the category is kept. The path names only fields the
/// schema knows (and array indices), except for an unknown field, whose
/// path ends in the request's own name: that one is reported unnamed.
fn json_error(path: &str, error: &serde_json::Error) -> InputError {
    let text = error.to_string();
    if text.starts_with("unknown field") {
        InputError::unknown_field()
    } else if let Some(rest) = text.strip_prefix("missing field `") {
        // The path points at the containing object; name the field itself.
        let name = rest.split('`').next().unwrap_or_default();
        let field = if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}.{name}")
        };
        InputError::new(field, "missing")
    } else if error.is_data() {
        InputError::new(path, "wrong type or value")
    } else {
        InputError::new(path, "malformed JSON")
    }
}

/// How a surface answers "not found". Each surface (voter page, voter API) has
/// exactly one such response, so unknown and malformed Ballot Tokens can't be
/// told apart.
pub trait NotFound {
    fn not_found() -> Response;
}

/// The Ballot Token from the `{token}` path segment, already sanitized. Any
/// failure (undecodable path, wrong format) is `S`'s uniform not-found response.
/// The token must be literal in the raw request path, so it has exactly one
/// spelling in a URL (and in logs or proxy rules that match it): any
/// percent-encoding, even of a valid token's own characters, is not found.
/// (The other segments of the Ballot routes are literal, as the router
/// matches them undecoded, and the base path has no `%`.)
pub struct BallotTokenPath<S> {
    pub token: BallotToken,
    surface: PhantomData<S>,
}

impl<St, S> FromRequestParts<St> for BallotTokenPath<S>
where
    St: Send + Sync,
    S: NotFound,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &St) -> Result<Self, Self::Rejection> {
        if raw_path(parts).contains('%') {
            return Err(S::not_found());
        }
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| S::not_found())?;
        let untrusted = UntrustedValue::from(raw);
        let token = untrusted
            .sanitize_value()
            .map_err(|_: Rejection| S::not_found())?;
        Ok(Self {
            token,
            surface: PhantomData,
        })
    }
}

/// The internal Ballot ID from the `{id}` path segment, already sanitized. Any
/// failure (undecodable path, wrong format) is `S`'s not-found response, the
/// same as for an unknown ID. The ID must be literal ASCII digits in the raw
/// request path: the router hands over the percent-decoded segment, so any
/// percent-encoding (e.g. `%31` for `1`) is caught on the raw path instead.
/// (The other segments of the admin Ballot routes are literal, as the router
/// matches them undecoded, so a `%` anywhere in the path is in the ID.)
pub struct BallotIdPath<S> {
    pub id: BallotId,
    surface: PhantomData<S>,
}

impl<St, S> FromRequestParts<St> for BallotIdPath<S>
where
    St: Send + Sync,
    S: NotFound,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &St) -> Result<Self, Self::Rejection> {
        if raw_path(parts).contains('%') {
            return Err(S::not_found());
        }
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| S::not_found())?;
        let id = UntrustedValue::from(raw)
            .sanitize_value()
            .map_err(|_: Rejection| S::not_found())?;
        Ok(Self {
            id,
            surface: PhantomData,
        })
    }
}

/// The request path as it arrived, before any nesting stripped a prefix:
/// still percent-encoded.
fn raw_path(parts: &Parts) -> &str {
    match parts.extensions.get::<OriginalUri>() {
        Some(OriginalUri(uri)) => uri.path(),
        None => parts.uri.path(),
    }
}

/// One name from an asset path segment (e.g. `/assets/logos/{name}`), still
/// untrusted. Must be literal in the raw request path: any percent-encoding,
/// an undecodable segment or a missing one is a plain 404, the same as an
/// unknown name.
pub struct AssetNamePath(pub UntrustedValue<String>);

impl<St> FromRequestParts<St> for AssetNamePath
where
    St: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &St) -> Result<Self, Self::Rejection> {
        if raw_path(parts).contains('%') {
            return Err(StatusCode::NOT_FOUND);
        }
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        Ok(Self(UntrustedValue::wrap(raw)))
    }
}

/// The version and file segments of an app module path
/// (`/assets/app/{version}/{file}`), still untrusted. Like
/// [`AssetNamePath`], they must be literal in the raw request path: any
/// percent-encoding or an undecodable segment is a plain 404.
pub struct AppModulePath {
    pub version: UntrustedValue<String>,
    pub file: UntrustedValue<String>,
}

impl<St> FromRequestParts<St> for AppModulePath
where
    St: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &St) -> Result<Self, Self::Rejection> {
        if raw_path(parts).contains('%') {
            return Err(StatusCode::NOT_FOUND);
        }
        let Path((version, file)) = Path::<(String, String)>::from_request_parts(parts, state)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        Ok(Self {
            version: UntrustedValue::wrap(version),
            file: UntrustedValue::wrap(file),
        })
    }
}

/// The raw query string of an app asset request (`?v=<version>`), still
/// untrusted; empty without one. Never rejects: any query still gets the
/// asset.
pub struct AssetQuery(pub UntrustedValue<String>);

impl<St> FromRequestParts<St> for AssetQuery
where
    St: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &St) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default().to_owned();
        Ok(Self(UntrustedValue::wrap(query)))
    }
}
