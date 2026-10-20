//! The JSON APIs' 409 Conflict responses, `{"error": <code>, "message":
//! <text>}`: built only here, with one fixed message per case, so the voter
//! and admin APIs can't phrase the same conflict differently by accident.
//! The message never names the Ballot, its Group or Voter Reference, or
//! anything the request sent.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::domain::ballot::{EditRefusal, Refusal};

/// Why a request conflicts with the Ballot as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Conflict {
    /// Voter API: the Ballot is Final and already has a Response, which
    /// can't be changed (`already_answered`).
    ResponseLocked,
    /// Admin API: the Ballot has a Response, so its content and settings
    /// can't change (`already_answered`).
    ///
    /// The same code as [`Self::ResponseLocked`], deliberately with another
    /// message: both say the Ballot is Answered, but the voter is told their
    /// Response is fixed and the admin that the Ballot can't be edited.
    EditLocked,
    /// Admin API: the Voting Deadline has passed without a Response and the
    /// Fallback has applied, so the Ballot's content and settings can't
    /// change until the deadline is extended (`fallback_applied`).
    FallbackApplied,
    /// Voter API: the Voting Deadline has passed; the Response can't be
    /// submitted or changed any more (`voting_closed`).
    VotingClosed,
    /// Voter API: the admin changed the Ballot since the voter fetched it
    /// (its revision moved on), or while the Response was being submitted;
    /// nothing was recorded, and the voter should fetch it again
    /// (`ballot_changed`).
    BallotChanged,
    /// Admin API: another Ballot in the Group already has this Voter
    /// Reference. Names neither the Group, the Voter Reference nor the other
    /// Ballot (`duplicate_voter_reference`).
    DuplicateVoterReference,
}

impl Conflict {
    /// The machine-readable `error` code.
    fn code(self) -> &'static str {
        match self {
            Self::ResponseLocked | Self::EditLocked => "already_answered",
            Self::FallbackApplied => "fallback_applied",
            Self::VotingClosed => "voting_closed",
            Self::BallotChanged => "ballot_changed",
            Self::DuplicateVoterReference => "duplicate_voter_reference",
        }
    }

    /// The human-readable `message`.
    fn message(self) -> &'static str {
        match self {
            Self::ResponseLocked => "this Ballot already has a Response, which can't be changed",
            Self::EditLocked => "this Ballot has a Response; it can no longer be edited",
            Self::FallbackApplied => {
                "the Fallback of this Ballot has applied; extend the Voting Deadline to edit it again"
            }
            Self::VotingClosed => {
                "the Voting Deadline has passed; no Response can be submitted or changed"
            }
            Self::BallotChanged => "the Ballot was just changed; fetch it again and resubmit",
            Self::DuplicateVoterReference => {
                "another Ballot in this Group already has this Voter Reference"
            }
        }
    }
}

impl IntoResponse for Conflict {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.code(), "message": self.message() });
        (StatusCode::CONFLICT, Json(body)).into_response()
    }
}

/// Why the voter's Response is refused.
impl From<Refusal> for Conflict {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::AlreadyAnswered => Self::ResponseLocked,
            Refusal::VotingClosed => Self::VotingClosed,
            Refusal::BallotChanged => Self::BallotChanged,
        }
    }
}

/// Why the admin's edit is refused.
impl From<EditRefusal> for Conflict {
    fn from(refusal: EditRefusal) -> Self {
        match refusal {
            EditRefusal::AlreadyAnswered => Self::EditLocked,
            EditRefusal::FallbackApplied => Self::FallbackApplied,
        }
    }
}
