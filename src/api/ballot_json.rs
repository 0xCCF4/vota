//! JSON pieces of a Ballot that the voter API and the admin API both show,
//! built in one place so the two can't drift apart: its Options and its
//! Response.

use serde_json::{Value, json};

use crate::api::rfc3339;
use crate::domain::ballot::{Ballot, BallotOption, OptionId, Response};

/// All of the Ballot's Options, as `{id, label}`, in the Ballot's order.
pub(super) fn options(ballot: &Ballot) -> Vec<Value> {
    ballot.options.iter().map(option).collect()
}

/// The Ballot's Options with these IDs, as `{id, label}`, in the Ballot's
/// order.
pub(super) fn options_with_ids(ballot: &Ballot, ids: &[OptionId]) -> Vec<Value> {
    ballot
        .options
        .iter()
        .filter(|o| ids.contains(&o.id))
        .map(option)
        .collect()
}

fn option(option: &BallotOption) -> Value {
    json!({ "id": option.id, "label": option.label })
}

/// The voter's Response: `{option_ids, answered_at, changed_at}`.
pub(super) fn response(response: &Response) -> Value {
    json!({
        "option_ids": response.option_ids,
        "answered_at": rfc3339(response.answered_at),
        "changed_at": rfc3339(response.changed_at),
    })
}
