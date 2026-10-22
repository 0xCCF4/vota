//! Voter requests: a Response, through the voter API or the no-JS form, with
//! the revision of the Ballot the voter saw.

use serde::Deserialize;
use untrusted_value::derive::UntrustedVariant;
use untrusted_value::{IntoUntrustedVariant as _, SanitizeWith as _, UntrustedValue};

use super::{InputError, Sanitized, base64url};
use crate::domain::ballot::{Ballot, OPTION_ID_BYTES, OptionId, SeenRevision, Selection};

/// `PUT /api/b/{token}/response` body as received.
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct ResponseRequest {
    option_ids: Vec<String>,
    /// Any JSON value; only a non-negative integer passes.
    revision: serde_json::Value,
}

/// A sanitized voter API Response: the selection and the revision of the
/// Ballot the voter made it on (the Ballot's current one).
pub struct ApiResponse {
    pub revision: SeenRevision,
    pub selection: Selection,
}

/// Why a voter API Response request was rejected.
#[derive(Debug)]
pub enum SelectionRejection {
    /// Malformed, duplicated, or breaking the Choice Mode; or a revision
    /// that is not a non-negative integer: a 422.
    Invalid(InputError),
    /// A well-formed Option ID that is not one of the Ballot's current
    /// Options, most likely one the admin has since replaced: the client
    /// should fetch the Ballot again. Says nothing about which ID, or
    /// whether it exists anywhere else.
    NotOnBallot,
    /// The revision is not the Ballot's current one: the admin changed the
    /// Ballot since the voter saw it, and the client should fetch it again.
    Stale,
}

/// Unboxes a voter API Response request against the Ballot it is for: the
/// revision first (it must be the Ballot's current one), then the selection.
pub fn api_response(
    request: UntrustedValue<ResponseRequest>,
    ballot: &Ballot,
) -> Result<ApiResponse, SelectionRejection> {
    let request: ResponseRequestUntrusted = request.to_untrusted_variant();
    let revision = request.revision.sanitize_with(|raw| {
        json_revision(raw).ok_or_else(|| {
            SelectionRejection::Invalid(InputError::new(
                "revision",
                "must be a non-negative integer",
            ))
        })
    })?;
    if !revision.is_current(ballot) {
        return Err(SelectionRejection::Stale);
    }
    let selection = request.option_ids.sanitize_with(|raw| {
        selection(raw, ballot).map_err(|(index, problem)| {
            if let SelectionProblem::NotAnOption = problem {
                return SelectionRejection::NotOnBallot;
            }
            let field = match index {
                Some(index) => format!("option_ids[{index}]"),
                None => "option_ids".to_owned(),
            };
            SelectionRejection::Invalid(InputError::new(field, problem.reason()))
        })
    })?;
    Ok(ApiResponse {
        revision,
        selection,
    })
}

/// A JSON integer from 0 to `i64::MAX` (no float such as `1.0`, string,
/// `null` or other type).
fn json_revision(raw: serde_json::Value) -> Option<SeenRevision> {
    let serde_json::Value::Number(number) = raw else {
        return None;
    };
    let revision = i64::try_from(number.as_u64()?).ok()?;
    Some(SeenRevision::new(revision, Sanitized::PROOF))
}

/// Plain decimal digits as the page writes them: no sign, no leading zero
/// (but `0`), no spaces, at most `i64::MAX`.
fn form_revision(raw: &str) -> Option<SeenRevision> {
    let canonical = !raw.is_empty()
        && raw.bytes().all(|b| b.is_ascii_digit())
        && (raw == "0" || !raw.starts_with('0'));
    if !canonical {
        return None;
    }
    let revision: i64 = raw.parse().ok()?;
    Some(SeenRevision::new(revision, Sanitized::PROOF))
}

/// `POST /b/{token}` form as received. Every field may repeat (see
/// [`super::UntrustedForm`]).
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct ResponseForm {
    /// The chosen Option IDs (radio buttons or hidden fields). Absent when
    /// nothing is ticked: the empty selection, valid only if the Ballot
    /// allows no Option (an unknown field instead makes the form
    /// unreadable, never empty).
    #[serde(default)]
    option: Vec<String>,
    /// `yes` from the confirmation step's submit button.
    #[serde(default)]
    confirm: Vec<String>,
    /// The Ballot's revision as the page showed it (a hidden field).
    #[serde(default)]
    revision: Vec<String>,
}

/// A sanitized Response from the no-JS voting form.
pub struct FormResponse {
    /// The revision of the Ballot the voter made it on (the Ballot's
    /// current one).
    pub revision: SeenRevision,
    pub selection: Selection,
    /// Whether the voter confirmed the selection on the confirmation step.
    pub confirmed: bool,
}

/// Why a Response from the no-JS voting form was rejected, so the page can name
/// the problem. Carries no submitted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormProblem {
    /// The number of selected Options breaks the Ballot's rules (none,
    /// unless it allows no Option; or more than one on a single-choice
    /// Ballot).
    WrongCount,
    /// A value is not an Option ID at all (malformed).
    UnknownOption,
    /// A well-formed Option ID that is not one of the Ballot's current
    /// Options, most likely one the admin has since replaced. Says nothing
    /// about whether it exists on another Ballot.
    NotOnBallot,
    /// The same Option was selected more than once.
    DuplicateOption,
    /// `confirm` is neither absent nor exactly one `yes`.
    InvalidConfirmation,
    /// The form itself could not be read (unknown field, invalid UTF-8).
    Unreadable,
    /// `revision` is not exactly one value of plain decimal digits.
    InvalidRevision,
    /// The revision is not the Ballot's current one: the admin changed the
    /// Ballot since the page was rendered.
    Stale,
}

/// Unboxes a Response from the no-JS voting form against the Ballot it is
/// for: the revision first (it must be the Ballot's current one), then the
/// selection and the confirmation.
pub fn form_response(
    request: UntrustedValue<ResponseForm>,
    ballot: &Ballot,
) -> Result<FormResponse, FormProblem> {
    let request: ResponseFormUntrusted = request.to_untrusted_variant();
    let revision = request.revision.sanitize_with(|raw| match raw.as_slice() {
        [one] => form_revision(one).ok_or(FormProblem::InvalidRevision),
        _ => Err(FormProblem::InvalidRevision),
    })?;
    if !revision.is_current(ballot) {
        return Err(FormProblem::Stale);
    }
    Ok(FormResponse {
        revision,
        selection: request.option.sanitize_with(|raw| {
            selection(raw, ballot).map_err(|(_, problem)| match problem {
                SelectionProblem::NotAnOptionId => FormProblem::UnknownOption,
                SelectionProblem::NotAnOption => FormProblem::NotOnBallot,
                SelectionProblem::Duplicate => FormProblem::DuplicateOption,
                SelectionProblem::Count(_) => FormProblem::WrongCount,
            })
        })?,
        confirmed: request.confirm.sanitize_with(confirmation)?,
    })
}

/// Absent, or exactly one `yes`.
fn confirmation(raw: Vec<String>) -> Result<bool, FormProblem> {
    match raw.as_slice() {
        [] => Ok(false),
        [yes] if yes == "yes" => Ok(true),
        _ => Err(FormProblem::InvalidConfirmation),
    }
}

/// Why a list of Option IDs is no valid selection of a Ballot's Options.
enum SelectionProblem {
    NotAnOptionId,
    NotAnOption,
    Duplicate,
    /// Breaks the Choice Mode; the reason from the domain rule.
    Count(&'static str),
}

impl SelectionProblem {
    fn reason(&self) -> &'static str {
        match self {
            Self::NotAnOptionId => "not an Option ID",
            Self::NotAnOption => "not an Option of this Ballot",
            Self::Duplicate => "duplicate Option",
            Self::Count(reason) => reason,
        }
    }
}

/// The Options in `raw` as a [`Selection`] of `ballot`, or the problem with
/// the value at an index (`None`: with the list as a whole).
fn selection(
    raw: Vec<String>,
    ballot: &Ballot,
) -> Result<Selection, (Option<usize>, SelectionProblem)> {
    // Malformed IDs and duplicates first: they are the client's mistake
    // whatever the Ballot looks like now.
    let mut option_ids: Vec<OptionId> = Vec::with_capacity(raw.len());
    for (index, id) in raw.into_iter().enumerate() {
        let error = |problem| (Some(index), problem);
        if !base64url::is_canonical(&id, OPTION_ID_BYTES) {
            return Err(error(SelectionProblem::NotAnOptionId));
        }
        let id = OptionId::new(id, Sanitized::PROOF);
        if option_ids.contains(&id) {
            return Err(error(SelectionProblem::Duplicate));
        }
        option_ids.push(id);
    }
    // Then membership: a well-formed ID that is not (or no longer) one of
    // this Ballot's Options. Before the count, as a Ballot whose Options
    // were replaced may have changed its Choice Mode too.
    if let Some(index) = option_ids
        .iter()
        .position(|id| !ballot.options.iter().any(|option| option.id == *id))
    {
        return Err((Some(index), SelectionProblem::NotAnOption));
    }
    ballot
        .check_selection_count(option_ids.len())
        .map_err(|reason| (None, SelectionProblem::Count(reason)))?;
    Ok(Selection::new(option_ids, Sanitized::PROOF))
}
