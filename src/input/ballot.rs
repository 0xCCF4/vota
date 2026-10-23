//! Admin requests about Ballots.

use std::collections::HashSet;

use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use untrusted_value::derive::UntrustedVariant;
use untrusted_value::{IntoUntrustedVariant as _, SanitizeWith as _, UntrustedValue};

use super::text::{Rejection, multi_line, single_line};
use super::theme::{accent_color, known_logo, known_theme};
use super::{InputError, Sanitized};
use crate::domain::ballot::{
    Ballot, BallotChanges, BallotFilter, BallotStatus, ChoiceMode, Description, EditScope,
    Extension, FallbackChoice, Group, NewBallot, OptionLabel, OptionLabels, SubmissionMode, Title,
    VoterReference, VotingDeadline,
};
use crate::domain::bulk::{BallotCount, Bulk, ListRejection, MAX_BALLOTS, VoterReferenceList};
use crate::domain::theme::{AccentColor, LogoName, ThemeCatalog, ThemeName};

/// `POST /admin/ballots` body as received.
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct CreateBallotRequest {
    title: String,
    /// Limited Markdown; absent means no description.
    #[serde(default, deserialize_with = "present")]
    description: Option<String>,
    options: Vec<OptionRequest>,
    /// Absent means the default; `null` is rejected like any non-string.
    #[serde(default, deserialize_with = "present")]
    choice_mode: Option<String>,
    /// Whether the voter may answer with no Option (multiple choice only);
    /// absent means `false`, `null` is rejected.
    #[serde(default, deserialize_with = "present")]
    allow_no_option: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    submission_mode: Option<String>,
    voting_deadline: String,
    /// Absent means the Voting Deadline plus the configured default gap.
    #[serde(default, deserialize_with = "present")]
    purge_date: Option<String>,
    /// Labels of the Fallback's Options; absent means no Fallback.
    #[serde(default, deserialize_with = "present")]
    fallback: Option<Vec<String>>,
    /// Absent means no Group.
    #[serde(default, deserialize_with = "present")]
    group: Option<String>,
    /// Absent means no Voter Reference; a list creates one Ballot per
    /// entry (bulk creation).
    #[serde(default, deserialize_with = "present")]
    voter_reference: Option<VoterReferences>,
    /// Bulk creation of this many Ballots with numbered Voter References;
    /// requires a Group, excludes `voter_reference`.
    #[serde(default, deserialize_with = "present")]
    count: Option<i64>,
    /// A Theme by name; absent means the default Theme, `null` is rejected.
    #[serde(default, deserialize_with = "present")]
    theme: Option<String>,
    /// `#rrggbb`; absent means the Theme's own accent colour.
    #[serde(default, deserialize_with = "present")]
    accent_color: Option<String>,
    /// A logo by name; absent means none.
    #[serde(default, deserialize_with = "present")]
    logo: Option<String>,
}

/// `voter_reference` at creation: one string, or a list of them.
#[derive(Deserialize)]
#[serde(untagged)]
enum VoterReferences {
    One(String),
    List(Vec<String>),
}

/// For optional fields that may be absent but not `null`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionRequest {
    label: String,
}

/// How a Voting Deadline or Purge Date given by the admin is read and where
/// it may lie: a local date-time (no offset) is in `zone`, the configured
/// `default_time_zone`; the instant must lie after `now`, and no later than
/// `now` plus the configured `max_date_horizon`.
#[derive(Debug, Clone, Copy)]
pub struct DateBounds {
    now: DateTime<Utc>,
    latest: DateTime<Utc>,
    zone: Tz,
}

impl DateBounds {
    pub fn new(now: DateTime<Utc>, max_date_horizon: std::time::Duration, zone: Tz) -> Self {
        let horizon = chrono::Duration::from_std(max_date_horizon).unwrap_or(chrono::Duration::MAX);
        let latest = now
            .checked_add_signed(horizon)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        Self { now, latest, zone }
    }

    /// The instant `raw` names: RFC 3339 with an offset, or a local
    /// date-time in the default time zone; then [`Self::check`]ed.
    fn instant(self, raw: &str) -> Result<DateTime<Utc>, Rejection> {
        let t = match DateTime::parse_from_rfc3339(raw) {
            Ok(t) => t.with_timezone(&Utc),
            Err(_) => local_in(raw, self.zone)?,
        };
        self.check(t)
    }

    /// `t` if it lies in the future and within the horizon.
    fn check(self, t: DateTime<Utc>) -> Result<DateTime<Utc>, Rejection> {
        if t <= self.now {
            return Err("must lie in the future");
        }
        if t > self.latest {
            return Err("must not lie further ahead than max_date_horizon allows");
        }
        Ok(t)
    }
}

/// Unboxes a create-Ballot request into the Ballot and, for a bulk
/// creation (a list of Voter References, or a `count`), how the Ballots get
/// their Voter References; the Ballot then has none of its own. `dates`
/// decides whether the Voting Deadline and Purge Date lie in the future and
/// within the horizon; `catalog` which Theme and logo names exist.
pub fn new_ballot(
    request: UntrustedValue<CreateBallotRequest>,
    dates: DateBounds,
    catalog: &dyn ThemeCatalog,
) -> Result<(NewBallot, Option<Bulk>), InputError> {
    let request: CreateBallotRequestUntrusted = request.to_untrusted_variant();
    let title = request.title.sanitize_with(title)?;
    let description = request.description.sanitize_with(description)?;
    let options = request.options.sanitize_with(option_labels)?;
    let choice_mode = request
        .choice_mode
        .sanitize_with(choice_mode)?
        .unwrap_or_default();
    let allow_no_option = request.allow_no_option.sanitize_with(|raw| {
        let allow = raw.unwrap_or(false);
        choice_mode
            .check_allow_no_option(allow)
            .map(|()| allow)
            .map_err(|reason| InputError::new("allow_no_option", reason))
    })?;
    let submission_mode = request
        .submission_mode
        .sanitize_with(submission_mode)?
        .unwrap_or_default();
    let voting_deadline = request
        .voting_deadline
        .sanitize_with(|raw| voting_deadline(raw, dates))?;
    let group = request.group.sanitize_with(group)?;
    let (voter_reference, bulk) = request.voter_reference.sanitize_with(voter_references)?;
    let has_reference = voter_reference.is_some() || bulk.is_some();
    let bulk = match request.count.sanitize_with(|raw| {
        raw.map(|raw| ballot_count(raw, group.is_some(), has_reference))
            .transpose()
    })? {
        Some(count) => Some(Bulk::Numbered(count)),
        None => bulk,
    };
    let ballot = NewBallot {
        title,
        description,
        choice_mode,
        allow_no_option,
        submission_mode,
        purge_date: request.purge_date.sanitize_with(|raw| {
            raw.map(|raw| purge_date(raw, voting_deadline.get(), dates))
                .transpose()
        })?,
        voting_deadline,
        fallback: request.fallback.sanitize_with(|raw| {
            let labels: Vec<&str> = options.iter().map(OptionLabel::as_str).collect();
            raw.map(|raw| fallback(raw, &labels, choice_mode))
                .transpose()
        })?,
        options,
        group,
        voter_reference,
        theme: request
            .theme
            .sanitize_with(|raw| raw.map(|raw| theme(raw, catalog)).transpose())?
            .unwrap_or_else(ThemeName::default_theme),
        accent_color: request
            .accent_color
            .sanitize_with(|raw| raw.map(accent).transpose())?,
        logo: request
            .logo
            .sanitize_with(|raw| raw.map(|raw| logo(raw, catalog)).transpose())?,
    };
    Ok((ballot, bulk))
}

/// `voter_reference` at creation: a string is the Ballot's own Voter
/// Reference, a list a bulk creation (each entry sanitized like a Voter
/// Reference, then the list's rules).
fn voter_references(
    raw: Option<VoterReferences>,
) -> Result<(Option<VoterReference>, Option<Bulk>), InputError> {
    match raw {
        None => Ok((None, None)),
        Some(VoterReferences::One(raw)) => Ok((voter_reference(Some(raw))?, None)),
        Some(VoterReferences::List(raw)) => {
            if raw.len() > MAX_BALLOTS {
                // Before sanitizing a thousand entries in vain.
                return Err(InputError::new(
                    "voter_reference",
                    ListRejection::TooMany.reason(),
                ));
            }
            let references = (raw.into_iter().enumerate())
                .map(|(index, raw)| {
                    optional_text(
                        raw,
                        &format!("voter_reference[{index}]"),
                        VoterReference::MAX_CHARS,
                        VoterReference::new,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let list =
                VoterReferenceList::new(references, Sanitized::PROOF).map_err(|rejection| {
                    let field = match rejection {
                        ListRejection::Duplicate(index) => format!("voter_reference[{index}]"),
                        ListRejection::Empty | ListRejection::TooMany => "voter_reference".into(),
                    };
                    InputError::new(field, rejection.reason())
                })?;
            Ok((None, Some(Bulk::Listed(list))))
        }
    }
}

/// `count`: needs a Group and no `voter_reference` (neither one nor a list).
fn ballot_count(raw: i64, has_group: bool, has_reference: bool) -> Result<BallotCount, InputError> {
    let error = |reason| InputError::new("count", reason);
    if has_reference {
        return Err(error("can't be combined with voter_reference"));
    }
    if !has_group {
        return Err(error("requires a group"));
    }
    BallotCount::new(raw).map_err(error)
}

fn description(raw: Option<String>) -> Result<Option<Description>, InputError> {
    raw.map(|raw| {
        multi_line(raw, Description::MAX_CHARS)
            .map(|text| Description::new(text, Sanitized::PROOF))
            .map_err(|reason| InputError::new("description", reason))
    })
    .transpose()
}

fn accent(raw: String) -> Result<AccentColor, InputError> {
    accent_color(raw).map_err(|reason| InputError::new("accent_color", reason))
}

fn logo(raw: String, catalog: &dyn ThemeCatalog) -> Result<LogoName, InputError> {
    known_logo(raw, catalog).map_err(|reason| InputError::new("logo", reason))
}

fn theme(raw: String, catalog: &dyn ThemeCatalog) -> Result<ThemeName, InputError> {
    known_theme(raw, catalog).map_err(|reason| InputError::new("theme", reason))
}

fn group(raw: Option<String>) -> Result<Option<Group>, InputError> {
    raw.map(|raw| optional_text(raw, "group", Group::MAX_CHARS, Group::new))
        .transpose()
}

fn voter_reference(raw: Option<String>) -> Result<Option<VoterReference>, InputError> {
    raw.map(|raw| {
        optional_text(
            raw,
            "voter_reference",
            VoterReference::MAX_CHARS,
            VoterReference::new,
        )
    })
    .transpose()
}

/// `PATCH /admin/ballots/{id}` body as received: any subset of the creation
/// fields. An absent field keeps its value; `null` is rejected like any other
/// wrong type.
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct EditBallotRequest {
    #[serde(default, deserialize_with = "present")]
    title: Option<String>,
    /// `null` removes the description.
    #[serde(default, deserialize_with = "nullable")]
    description: Option<Option<String>>,
    /// Replaces all Options.
    #[serde(default, deserialize_with = "present")]
    options: Option<Vec<OptionRequest>>,
    #[serde(default, deserialize_with = "present")]
    choice_mode: Option<String>,
    /// `null` is rejected; `false` turns it off.
    #[serde(default, deserialize_with = "present")]
    allow_no_option: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    submission_mode: Option<String>,
    /// Labels of the Fallback's Options; `null` removes the Fallback.
    #[serde(default, deserialize_with = "nullable")]
    fallback: Option<Option<Vec<String>>>,
    /// `null` removes the Group.
    #[serde(default, deserialize_with = "nullable")]
    group: Option<Option<String>>,
    /// `null` removes the Voter Reference.
    #[serde(default, deserialize_with = "nullable")]
    voter_reference: Option<Option<String>>,
    /// Must lie in the future. Moving it past the Purge Date shifts the Purge
    /// Date unless one is given too.
    #[serde(default, deserialize_with = "present")]
    voting_deadline: Option<String>,
    /// Must lie in the future and not before the Voting Deadline.
    #[serde(default, deserialize_with = "present")]
    purge_date: Option<String>,
    /// A Theme by name; `null` is rejected (name `classic` for the default).
    #[serde(default, deserialize_with = "present")]
    theme: Option<String>,
    /// `null` removes the accent colour (the Theme's own applies).
    #[serde(default, deserialize_with = "nullable")]
    accent_color: Option<Option<String>>,
    /// `null` removes the logo.
    #[serde(default, deserialize_with = "nullable")]
    logo: Option<Option<String>>,
}

impl EditBallotRequest {
    /// What the edit touches, by the fields present (`null` counts as
    /// present): only `group` and/or `voter_reference` is bookkeeping.
    fn scope(&self) -> EditScope {
        let content = self.title.is_some()
            || self.description.is_some()
            || self.options.is_some()
            || self.choice_mode.is_some()
            || self.allow_no_option.is_some()
            || self.submission_mode.is_some()
            || self.fallback.is_some()
            || self.voting_deadline.is_some()
            || self.purge_date.is_some()
            || self.theme.is_some()
            || self.accent_color.is_some()
            || self.logo.is_some();
        if content {
            EditScope::Content
        } else {
            EditScope::Bookkeeping
        }
    }
}

/// Which fields an edit names, before any of their values is looked at, so
/// the Ballot store can refuse it first (e.g. once the Ballot is Answered).
/// Reads only the presence of fields; the request stays untrusted.
pub fn edit_scope(
    request: UntrustedValue<EditBallotRequest>,
) -> (EditScope, UntrustedValue<EditBallotRequest>) {
    let peeked: Result<_, std::convert::Infallible> =
        request.sanitize_with(|raw| Ok((raw.scope(), UntrustedValue::wrap(raw))));
    match peeked {
        Ok(peeked) => peeked,
    }
}

/// For optional fields that may be absent (`None`) or `null`
/// (`Some(None)`), e.g. to clear a value.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Unboxes an edit of `ballot` into the whole Ballot as the edit leaves it,
/// validated like a new Ballot: absent fields keep their current values, and
/// the rules apply to the result (e.g. the current Fallback must still obey a
/// changed Choice Mode). `dates` decides which dates are acceptable,
/// `catalog` which Theme and logo names exist.
pub fn ballot_changes(
    request: UntrustedValue<EditBallotRequest>,
    ballot: &Ballot,
    dates: DateBounds,
    catalog: &dyn ThemeCatalog,
) -> Result<BallotChanges, InputError> {
    let request: EditBallotRequestUntrusted = request.to_untrusted_variant();
    let title = request
        .title
        .sanitize_with(|raw| raw.map(title).transpose())?
        .unwrap_or_else(|| Title::from_store(ballot.title.clone()));
    let description = request
        .description
        .sanitize_with(|raw| raw.map(description).transpose())?
        .unwrap_or_else(|| ballot.description.clone().map(Description::from_store));
    let options = request
        .options
        .sanitize_with(|raw| raw.map(option_labels).transpose())?;
    let choice_mode = request
        .choice_mode
        .sanitize_with(choice_mode)?
        .unwrap_or(ballot.choice_mode);
    let allow_no_option = request
        .allow_no_option
        .sanitize_with(|raw| allowed_no_option(raw, ballot.allow_no_option, choice_mode))?;
    let submission_mode = request
        .submission_mode
        .sanitize_with(submission_mode)?
        .unwrap_or(ballot.submission_mode);
    let voting_deadline = request
        .voting_deadline
        .sanitize_with(|raw| raw.map(|raw| voting_deadline(raw, dates)).transpose())?
        .map_or(ballot.voting_deadline, VotingDeadline::get);
    let purge_date = request
        .purge_date
        .sanitize_with(|raw| {
            raw.map(|raw| purge_date(raw, voting_deadline, dates))
                .transpose()
        })?
        .map_or_else(|| shifted_purge_date(ballot, voting_deadline, dates), Ok)?;
    // The Option labels as the edit leaves them.
    let labels: Vec<&str> = match &options {
        Some(options) => options.iter().map(OptionLabel::as_str).collect(),
        None => ballot.options.iter().map(|o| o.label.as_str()).collect(),
    };
    let fallback = match request.fallback.sanitize_with(|raw| {
        raw.map(|raw| {
            raw.map(|raw| fallback(raw, &labels, choice_mode))
                .transpose()
        })
        .transpose()
    })? {
        Some(fallback) => fallback,
        None => kept_fallback(ballot, &labels, choice_mode)?,
    };
    let group = request
        .group
        .sanitize_with(|raw| raw.map(group).transpose())?
        .unwrap_or_else(|| ballot.group.clone().map(Group::from_store));
    let voter_reference = request
        .voter_reference
        .sanitize_with(|raw| raw.map(voter_reference).transpose())?
        .unwrap_or_else(|| {
            let current = ballot.voter_reference.clone();
            current.map(VoterReference::from_store)
        });
    let theme = request
        .theme
        .sanitize_with(|raw| raw.map(|raw| theme(raw, catalog)).transpose())?
        .unwrap_or_else(|| ThemeName::from_store(ballot.theme.clone()));
    let accent_color = request
        .accent_color
        .sanitize_with(|raw| raw.map(|raw| raw.map(accent).transpose()).transpose())?
        .unwrap_or_else(|| ballot.accent_color.clone());
    let logo = request
        .logo
        .sanitize_with(|raw| {
            raw.map(|raw| raw.map(|raw| logo(raw, catalog)).transpose())
                .transpose()
        })?
        .unwrap_or_else(|| ballot.logo.clone().map(LogoName::from_store));
    Ok(BallotChanges {
        title,
        description,
        options,
        choice_mode,
        allow_no_option,
        submission_mode,
        fallback,
        group,
        voter_reference,
        voting_deadline,
        purge_date,
        theme,
        accent_color,
        logo,
    })
}

/// Allow No Option as an edit leaves it (`raw`, or else `current`), checked
/// against the resulting Choice Mode. A Choice Mode changed to single while
/// it stays on is the Choice Mode's fault: the admin must turn it off too.
fn allowed_no_option(
    raw: Option<bool>,
    current: bool,
    choice_mode: ChoiceMode,
) -> Result<bool, InputError> {
    let allow = raw.unwrap_or(current);
    choice_mode.check_allow_no_option(allow).map_err(|reason| {
        if raw.is_some() {
            InputError::new("allow_no_option", reason)
        } else {
            InputError::new(
                "choice_mode",
                "the Ballot allows no Option, which needs multiple choice; \
                 set allow_no_option to false as well",
            )
        }
    })?;
    Ok(allow)
}

/// The Purge Date when an edit moves the Voting Deadline without giving one:
/// shifted by [`Ballot::purge_date_for_deadline`], but, unlike an extension's,
/// capped by the horizon. A shift past it is the moved deadline's fault, so
/// the error names `voting_deadline`; the admin can give a `purge_date` too.
fn shifted_purge_date(
    ballot: &Ballot,
    voting_deadline: DateTime<Utc>,
    dates: DateBounds,
) -> Result<DateTime<Utc>, InputError> {
    let purge_date = ballot.purge_date_for_deadline(voting_deadline);
    if purge_date != ballot.purge_date && purge_date > dates.latest {
        return Err(InputError::new(
            "voting_deadline",
            "would move the Purge Date further ahead than max_date_horizon allows; give a purge_date as well",
        ));
    }
    Ok(purge_date)
}

/// `POST /admin/ballots/{id}/extend` body as received.
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct ExtendRequest {
    voting_deadline: String,
}

/// Unboxes an extension of `ballot`'s Voting Deadline: the new deadline must
/// lie in the future, within the horizon and after the current one. The Purge
/// Date follows it ([`Ballot::purge_date_for_deadline`]), uncapped: it is
/// computed, not given.
pub fn extension(
    request: UntrustedValue<ExtendRequest>,
    ballot: &Ballot,
    dates: DateBounds,
) -> Result<Extension, InputError> {
    let request: ExtendRequestUntrusted = request.to_untrusted_variant();
    let deadline = request
        .voting_deadline
        .sanitize_with(|raw| voting_deadline(raw, dates))?
        .get();
    if deadline <= ballot.voting_deadline {
        return Err(InputError::new(
            "voting_deadline",
            "must be later than the current Voting Deadline",
        ));
    }
    Ok(Extension {
        voting_deadline: deadline,
        purge_date: ballot.purge_date_for_deadline(deadline),
    })
}

/// The Ballot's current Fallback, if any, resolved by label against the
/// edited Option labels (`labels`) and checked against the edited Choice Mode.
/// Rejected if one of its Options is gone: the admin must then give the
/// Fallback again (or `null`).
fn kept_fallback(
    ballot: &Ballot,
    labels: &[&str],
    choice_mode: ChoiceMode,
) -> Result<Option<FallbackChoice>, InputError> {
    let Some(ids) = &ballot.fallback else {
        return Ok(None);
    };
    let positions = ballot
        .options
        .iter()
        .filter(|option| ids.contains(&option.id))
        .map(|option| labels.iter().position(|label| *label == option.label))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            InputError::new(
                "fallback",
                "the Fallback's Options are no longer all Options of this Ballot; give the Fallback again",
            )
        })?;
    fallback_choice(positions, choice_mode).map(Some)
}

/// `GET /admin/ballots` query as received. Every parameter may repeat (see
/// [`super::UntrustedQuery`]); each is allowed at most once.
#[derive(Deserialize, UntrustedVariant)]
#[serde(deny_unknown_fields)]
pub struct ListBallotsQuery {
    #[serde(default)]
    group: Vec<String>,
    #[serde(default)]
    status: Vec<String>,
}

/// Unboxes the admin list's filters. A Group filter is sanitized exactly like
/// a Group at creation, so it matches the stored, normalised form.
pub fn ballot_filter(query: UntrustedValue<ListBallotsQuery>) -> Result<BallotFilter, InputError> {
    let query: ListBallotsQueryUntrusted = query.to_untrusted_variant();
    Ok(BallotFilter {
        group: query.group.sanitize_with(|raw| {
            at_most_one(raw, "group")?
                .map(|raw| optional_text(raw, "group", Group::MAX_CHARS, Group::new))
                .transpose()
        })?,
        status: query.status.sanitize_with(|raw| {
            at_most_one(raw, "status")?
                .map(|name| {
                    BallotStatus::from_name(&name).ok_or_else(|| {
                        let names = BallotStatus::ALL.map(|s| format!("\"{}\"", s.as_str()));
                        InputError::new("status", format!("must be one of {}", names.join(", ")))
                    })
                })
                .transpose()
        })?,
    })
}

/// The single value of a parameter that may be given at most once.
fn at_most_one(raw: Vec<String>, field: &str) -> Result<Option<String>, InputError> {
    let mut values = raw.into_iter();
    match (values.next(), values.next()) {
        (value, None) => Ok(value),
        (_, Some(_)) => Err(InputError::new(field, "must be given at most once")),
    }
}

/// An enum by exact name (no trimming, no case folding), if present.
fn enum_name<T>(
    raw: Option<String>,
    field: &str,
    from_name: impl Fn(&str) -> Option<T>,
    names: &[&str],
) -> Result<Option<T>, InputError> {
    raw.map(|name| {
        from_name(&name).ok_or_else(|| {
            let quoted: Vec<_> = names.iter().map(|n| format!("\"{n}\"")).collect();
            InputError::new(field, format!("must be one of {}", quoted.join(", ")))
        })
    })
    .transpose()
}

fn choice_mode(raw: Option<String>) -> Result<Option<ChoiceMode>, InputError> {
    enum_name(
        raw,
        "choice_mode",
        ChoiceMode::from_name,
        &ChoiceMode::ALL.map(ChoiceMode::as_str),
    )
}

fn submission_mode(raw: Option<String>) -> Result<Option<SubmissionMode>, InputError> {
    enum_name(
        raw,
        "submission_mode",
        SubmissionMode::from_name,
        &SubmissionMode::ALL.map(SubmissionMode::as_str),
    )
}

fn title(raw: String) -> Result<Title, InputError> {
    single_line(raw, Title::MAX_CHARS)
        .map(|text| Title::new(text, Sanitized::PROOF))
        .map_err(|reason| InputError::new("title", reason))
}

/// Single-line text for an optional field that is present.
fn optional_text<T>(
    raw: String,
    field: &str,
    max_chars: usize,
    new: fn(String, Sanitized) -> T,
) -> Result<T, InputError> {
    single_line(raw, max_chars)
        .map(|text| new(text, Sanitized::PROOF))
        .map_err(|reason| InputError::new(field, reason))
}

fn option_labels(raw: Vec<OptionRequest>) -> Result<OptionLabels, InputError> {
    if raw.len() < OptionLabels::MIN {
        return Err(InputError::new(
            "options",
            "at least two Options are required",
        ));
    }
    let mut seen = HashSet::new();
    let mut labels = Vec::with_capacity(raw.len());
    for (index, option) in raw.into_iter().enumerate() {
        let field = || format!("options[{index}].label");
        let label = single_line(option.label, OptionLabel::MAX_CHARS)
            .map_err(|reason| InputError::new(field(), reason))?;
        if !seen.insert(label.clone()) {
            return Err(InputError::new(field(), "duplicate Option label"));
        }
        labels.push(OptionLabel::new(label, Sanitized::PROOF));
    }
    Ok(OptionLabels::new(labels, Sanitized::PROOF))
}

/// A Fallback given by Option labels: each label, sanitized like an Option
/// label, must match one of `options` (the Ballot's Option labels, in order)
/// exactly, at most once; the set obeys `choice_mode` (so it is never empty).
fn fallback(
    raw: Vec<String>,
    options: &[&str],
    choice_mode: ChoiceMode,
) -> Result<FallbackChoice, InputError> {
    let mut positions = Vec::with_capacity(raw.len());
    for (index, label) in raw.into_iter().enumerate() {
        let error = |reason| InputError::new(format!("fallback[{index}]"), reason);
        let label = single_line(label, OptionLabel::MAX_CHARS).map_err(error)?;
        let position = options
            .iter()
            .position(|option| *option == label)
            .ok_or_else(|| error("not an Option of this Ballot"))?;
        if positions.contains(&position) {
            return Err(error("duplicate Option"));
        }
        positions.push(position);
    }
    fallback_choice(positions, choice_mode)
}

/// Fallback positions as a [`FallbackChoice`], if they obey `choice_mode`.
fn fallback_choice(
    mut positions: Vec<usize>,
    choice_mode: ChoiceMode,
) -> Result<FallbackChoice, InputError> {
    choice_mode
        .check_selection_count(positions.len())
        .map_err(|reason| InputError::new("fallback", reason))?;
    positions.sort_unstable();
    Ok(FallbackChoice::new(positions, Sanitized::PROOF))
}

/// A Voting Deadline: in the future and within the horizon.
fn voting_deadline(raw: String, dates: DateBounds) -> Result<VotingDeadline, InputError> {
    let error = |reason| InputError::new("voting_deadline", reason);
    let deadline = dates.instant(&raw).map_err(error)?;
    Ok(VotingDeadline::new(deadline, Sanitized::PROOF))
}

/// A Purge Date: in the future, within the horizon and not before the
/// (resulting) Voting Deadline.
fn purge_date(
    raw: String,
    voting_deadline: DateTime<Utc>,
    dates: DateBounds,
) -> Result<DateTime<Utc>, InputError> {
    let error = |reason| InputError::new("purge_date", reason);
    let purge_date = dates.instant(&raw).map_err(error)?;
    if purge_date < voting_deadline {
        return Err(error("must not be before the Voting Deadline"));
    }
    Ok(purge_date)
}

/// The instant a local date-time without offset names in `zone`:
/// exactly `YYYY-MM-DDTHH:MM`, optionally with `:SS` and then optionally a
/// fraction (`.` and 1–9 digits), as RFC 3339 writes them. A time a clock
/// change skips or repeats in `zone` is refused, asking for an offset.
fn local_in(raw: &str, zone: Tz) -> Result<DateTime<Utc>, Rejection> {
    const MALFORMED: Rejection = "not an RFC 3339 timestamp, or a local date-time in the \
                                  server's default time zone (YYYY-MM-DDTHH:MM[:SS])";
    // The exact shape first: chrono's own parsing would also take e.g. a
    // one-digit month or a sign before the year.
    let shape = raw.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 => b == b'-',
        10 => b == b'T',
        13 | 16 => b == b':',
        19 => b == b'.',
        _ => b.is_ascii_digit(),
    });
    let format = match raw.len() {
        16 => "%Y-%m-%dT%H:%M",
        19 => "%Y-%m-%dT%H:%M:%S",
        21..=29 => "%Y-%m-%dT%H:%M:%S%.f",
        _ => return Err(MALFORMED),
    };
    if !shape {
        return Err(MALFORMED);
    }
    let local = NaiveDateTime::parse_from_str(raw, format).map_err(|_| MALFORMED)?;
    match zone.from_local_datetime(&local) {
        LocalResult::Single(t) => Ok(t.with_timezone(&Utc)),
        LocalResult::None => Err(
            "does not exist in the server's default time zone (skipped by a clock change); \
             give a UTC offset",
        ),
        LocalResult::Ambiguous(..) => Err(
            "is ambiguous in the server's default time zone (repeated by a clock change); \
             give a UTC offset",
        ),
    }
}
