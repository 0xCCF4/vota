//! The Ballot aggregate, its validated value types and its rules.
//!
//! The value types ([`Title`], [`Description`], [`OptionLabel`], [`OptionLabels`],
//! [`VotingDeadline`], [`BallotToken`]) can only be built from outside data by
//! the sanitizers in `crate::input`: their constructors demand an
//! `input::Sanitized` proof, which only that module can create. An
//! [`OptionId`] is built by those sanitizers too, and otherwise only by the
//! Ballot store (generated for a new Option, or loaded back).

use chrono::{DateTime, Utc};

use crate::domain::theme::{AccentColor, LogoName, ThemeName};
use crate::domain::token_generator::random_base64url;
use crate::input::Sanitized;

/// A Ballot's title: plain single-line text, 1..=200 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

impl Title {
    pub const MAX_CHARS: usize = 200;

    pub(crate) fn new(text: String, _: Sanitized) -> Self {
        Self(text)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Ballot's description: limited Markdown source, 1..=10 000 characters,
/// lines separated by `\n`. Stored and returned as source; rendered to
/// sanitised HTML only for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description(String);

impl Description {
    pub const MAX_CHARS: usize = 10_000;

    pub(crate) fn new(text: String, _: Sanitized) -> Self {
        Self(text)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An Option's label: plain single-line text, 1..=200 characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OptionLabel(String);

impl OptionLabel {
    pub const MAX_CHARS: usize = 200;

    pub(crate) fn new(text: String, _: Sanitized) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The Options of a new Ballot, in order: at least two, labels unique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionLabels(Vec<OptionLabel>);

impl OptionLabels {
    pub const MIN: usize = 2;

    pub(crate) fn new(labels: Vec<OptionLabel>, _: Sanitized) -> Self {
        Self(labels)
    }

    pub fn iter(&self) -> impl Iterator<Item = &OptionLabel> {
        self.0.iter()
    }
}

/// A Group: optional free-text label shared by several Ballots so their
/// results can be fetched together; single-line text, 1..=200 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group(String);

impl Group {
    pub const MAX_CHARS: usize = 200;

    pub(crate) fn new(text: String, _: Sanitized) -> Self {
        Self(text)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Voter Reference: the admin's hidden note of whom a Ballot belongs to,
/// unique within its Group; single-line text, 1..=200 characters. Never shown
/// to the voter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoterReference(String);

impl VoterReference {
    pub const MAX_CHARS: usize = 200;

    pub(crate) fn new(text: String, _: Sanitized) -> Self {
        Self(text)
    }

    /// A value read back from the Ballot store, which only ever holds
    /// sanitized ones.
    pub(crate) fn from_store(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Voting Deadline (UTC) that lay in the future when it was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VotingDeadline(DateTime<Utc>);

impl VotingDeadline {
    pub(crate) fn new(deadline: DateTime<Utc>, _: Sanitized) -> Self {
        Self(deadline)
    }

    pub fn get(self) -> DateTime<Utc> {
        self.0
    }
}

/// A Ballot's internal ID, as admins refer to it: a positive integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallotId(i64);

impl BallotId {
    pub(crate) fn new(id: i64, _: Sanitized) -> Self {
        Self(id)
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

/// A Ballot Token: 128 CSPRNG bits as 22 base64url characters without padding.
/// The voter's only credential.
#[derive(Clone, PartialEq, Eq)]
pub struct BallotToken(String);

impl BallotToken {
    pub const BYTES: usize = 16;
    pub const CHARS: usize = 22;

    /// A fresh token from the Token generator.
    pub(crate) fn generate() -> Self {
        Self(random_base64url(Self::BYTES))
    }

    pub(crate) fn new(token: String, _: Sanitized) -> Self {
        Self(token)
    }

    /// A token read back from the Ballot store, which only ever holds
    /// generated ones.
    pub(crate) fn from_store(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BallotToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BallotToken(<redacted>)")
    }
}

/// Random bytes in a server-generated Option ID (11 base64url characters).
pub const OPTION_ID_BYTES: usize = 8;

/// An Option ID: generated by the server, 64 CSPRNG bits as 11 base64url
/// characters. Built only by the Ballot store (fresh, or loaded back) and by
/// the sanitizers in `crate::input`, so an `OptionId` is always one the app
/// issued or one that has the form of one. Serialized as its plain string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OptionId(String);

impl OptionId {
    /// A fresh Option ID from the Token generator, for the Ballot store.
    pub(crate) fn generate() -> Self {
        Self(random_base64url(OPTION_ID_BYTES))
    }

    pub(crate) fn new(id: String, _: Sanitized) -> Self {
        Self(id)
    }

    /// An ID read back from the Ballot store, which only ever holds
    /// generated ones.
    pub(crate) fn from_store(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for OptionId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Whether a Ballot is single choice (exactly one Option) or multiple choice
/// (one or more Options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChoiceMode {
    #[default]
    Single,
    Multiple,
}

impl ChoiceMode {
    pub const ALL: [Self; 2] = [Self::Single, Self::Multiple];

    /// Whether a selection of `count` distinct Options obeys this Choice
    /// Mode: exactly one Option when single choice, one or more when multiple
    /// choice; never empty. Responses and Fallbacks obey the same rule.
    pub fn check_selection_count(self, count: usize) -> Result<(), &'static str> {
        match (self, count) {
            (_, 0) => Err("select an Option"),
            (Self::Single, 1) | (Self::Multiple, _) => Ok(()),
            (Self::Single, _) => Err("single choice: select exactly one Option"),
        }
    }

    /// The wire and storage name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multiple => "multiple",
        }
    }

    /// The mode with this wire or storage name (exact match).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// Whether a Ballot's Response is Final (submitted once, then locked) or
/// Editable (can be changed until the Voting Deadline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmissionMode {
    #[default]
    Final,
    Editable,
}

impl SubmissionMode {
    pub const ALL: [Self; 2] = [Self::Final, Self::Editable];

    /// The wire and storage name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Editable => "editable",
        }
    }

    /// The mode with this wire or storage name (exact match).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// The Fallback of a new or edited Ballot, as positions in its Options
/// (ascending, distinct): non-empty and obeying the Ballot's Choice Mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackChoice(Vec<usize>);

impl FallbackChoice {
    pub(crate) fn new(positions: Vec<usize>, _: Sanitized) -> Self {
        Self(positions)
    }

    pub fn positions(&self) -> &[usize] {
        &self.0
    }
}

/// What the admin asked for when creating a Ballot, validated.
#[derive(Debug, Clone)]
pub struct NewBallot {
    pub title: Title,
    pub description: Option<Description>,
    pub options: OptionLabels,
    pub choice_mode: ChoiceMode,
    pub submission_mode: SubmissionMode,
    pub voting_deadline: VotingDeadline,
    /// `None`: the default, the Voting Deadline plus the configured gap.
    /// Never before the Voting Deadline.
    pub purge_date: Option<DateTime<Utc>>,
    pub fallback: Option<FallbackChoice>,
    pub group: Option<Group>,
    pub voter_reference: Option<VoterReference>,
    /// The default Theme unless the admin named one.
    pub theme: ThemeName,
    pub accent_color: Option<AccentColor>,
    pub logo: Option<LogoName>,
}

/// Which Ballots the admin wants listed; `None` means any.
#[derive(Debug, Clone, Default)]
pub struct BallotFilter {
    pub group: Option<Group>,
    /// Compared with the Ballot Status derived at the time of listing.
    pub status: Option<BallotStatus>,
}

/// A new Ballot with every default resolved, ready to be stored.
#[derive(Debug, Clone)]
pub struct BallotDraft {
    pub title: Title,
    pub description: Option<Description>,
    pub options: OptionLabels,
    pub choice_mode: ChoiceMode,
    pub submission_mode: SubmissionMode,
    pub voting_deadline: DateTime<Utc>,
    pub purge_date: DateTime<Utc>,
    pub fallback: Option<FallbackChoice>,
    pub group: Option<Group>,
    pub voter_reference: Option<VoterReference>,
    pub theme: ThemeName,
    pub accent_color: Option<AccentColor>,
    pub logo: Option<LogoName>,
}

impl BallotDraft {
    /// Resolves defaults: unless given, the Purge Date is the Voting Deadline
    /// plus the configured gap.
    pub fn new(new: NewBallot, default_purge_gap: std::time::Duration) -> Self {
        let voting_deadline = new.voting_deadline.get();
        let purge_date = new.purge_date.unwrap_or_else(|| {
            let gap =
                chrono::Duration::from_std(default_purge_gap).unwrap_or(chrono::Duration::MAX);
            voting_deadline
                .checked_add_signed(gap)
                .unwrap_or(DateTime::<Utc>::MAX_UTC)
        });
        Self {
            title: new.title,
            description: new.description,
            options: new.options,
            choice_mode: new.choice_mode,
            submission_mode: new.submission_mode,
            voting_deadline,
            purge_date,
            fallback: new.fallback,
            group: new.group,
            voter_reference: new.voter_reference,
            theme: new.theme,
            accent_color: new.accent_color,
            logo: new.logo,
        }
    }
}

/// A Ballot as an admin edit leaves it, validated as a whole: every field
/// holds its resulting value, whether the edit changed it or not.
#[derive(Debug, Clone)]
pub struct BallotChanges {
    pub title: Title,
    pub description: Option<Description>,
    /// `Some`: the new Options, replacing all current ones (with new Option
    /// IDs); `None`: the current Options stay as they are.
    pub options: Option<OptionLabels>,
    pub choice_mode: ChoiceMode,
    pub submission_mode: SubmissionMode,
    /// As positions in the resulting Options.
    pub fallback: Option<FallbackChoice>,
    pub group: Option<Group>,
    pub voter_reference: Option<VoterReference>,
    pub voting_deadline: DateTime<Utc>,
    /// Never before `voting_deadline`.
    pub purge_date: DateTime<Utc>,
    /// Unchanged by the edit, possibly a Theme that has gone since; changed,
    /// one that exists.
    pub theme: ThemeName,
    pub accent_color: Option<AccentColor>,
    /// Unchanged by the edit, possibly a logo that has gone since; changed,
    /// one that exists.
    pub logo: Option<LogoName>,
}

/// A Ballot's new Voting Deadline and the Purge Date that goes with it, after
/// the admin extended the deadline.
#[derive(Debug, Clone, Copy)]
pub struct Extension {
    pub voting_deadline: DateTime<Utc>,
    /// Never before `voting_deadline`.
    pub purge_date: DateTime<Utc>,
}

/// A stored Ballot.
#[derive(Debug, Clone)]
pub struct Ballot {
    /// Internal ID; admins refer to Ballots by it.
    pub id: i64,
    pub token: BallotToken,
    pub title: String,
    /// Markdown source, as sanitised at the boundary.
    pub description: Option<String>,
    pub options: Vec<BallotOption>,
    pub choice_mode: ChoiceMode,
    pub submission_mode: SubmissionMode,
    pub voting_deadline: DateTime<Utc>,
    pub purge_date: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    /// The voter's Response, if they submitted one.
    pub response: Option<Response>,
    /// IDs of the Fallback's Options in the Ballot's Option order, if the
    /// admin set a Fallback (never empty). Shown to the voter from the start,
    /// so they know what is recorded if they don't answer.
    pub fallback: Option<Vec<OptionId>>,
    /// Admin business, never shown to the voter.
    pub group: Option<String>,
    /// Admin business, never shown to the voter.
    pub voter_reference: Option<String>,
    /// The Theme's name as the admin chose it. The Theme may have gone since
    /// (removed from the theme folder); the voting page then uses the
    /// default Theme.
    pub theme: String,
    /// Theme Tweak: replaces the Theme's own accent colour.
    pub accent_color: Option<AccentColor>,
    /// Theme Tweak: the logo's name. The logo may have gone since (removed
    /// from the logo folder); the voting page then shows none.
    pub logo: Option<String>,
    /// Bumped by every admin edit of the Ballot's content
    /// ([`EditScope::Content`]). The voter sees it and sends it back with a
    /// Response ([`SeenRevision`]), which is only recorded if the Ballot
    /// still stands as the voter saw it. Bookkeeping edits (Group, Voter
    /// Reference) don't bump it: the voter never sees them.
    pub revision: i64,
}

impl Ballot {
    /// Whether voting is still open at `now`. The Voting Deadline is
    /// exclusive: at the deadline itself voting is closed.
    pub fn is_open(&self, now: DateTime<Utc>) -> bool {
        now < self.voting_deadline
    }

    /// Whether the voter may submit a Response at `now`: never once voting
    /// has closed; before that, a Final Ballot accepts one Response and then
    /// no more (the Ballot store's conditional write enforces both
    /// atomically); an Editable Ballot accepts any number, the last one wins.
    pub fn accepts_response(&self, now: DateTime<Utc>) -> Result<(), Refusal> {
        if !self.is_open(now) {
            return Err(Refusal::VotingClosed);
        }
        match (self.submission_mode, &self.response) {
            (SubmissionMode::Final, Some(_)) => Err(Refusal::AlreadyAnswered),
            (SubmissionMode::Final, None) | (SubmissionMode::Editable, _) => Ok(()),
        }
    }

    /// Whether the admin may make an edit of this scope at `now`: content and
    /// settings only while the Ballot has no Response, so a voter's Response
    /// never silently changes meaning, and no applied Fallback (Auto-fallback),
    /// so a recorded result doesn't change either (extending the deadline
    /// reopens the Ballot first); the admin's bookkeeping (Group, Voter
    /// Reference) at any time, as the voter never sees it. (The Ballot store's
    /// conditional write enforces this atomically.)
    pub fn accepts_edit(&self, scope: EditScope, now: DateTime<Utc>) -> Result<(), EditRefusal> {
        if scope == EditScope::Bookkeeping {
            return Ok(());
        }
        match self.status(now) {
            BallotStatus::Answered => Err(EditRefusal::AlreadyAnswered),
            BallotStatus::AutoFallback => Err(EditRefusal::FallbackApplied),
            BallotStatus::Unanswered => Ok(()),
        }
    }

    /// The Purge Date once the Voting Deadline moves to `new_deadline`: the
    /// current one while it is not before `new_deadline`; otherwise
    /// `new_deadline` plus the current gap between Voting Deadline and Purge
    /// Date, so the Purge Date is never before the Voting Deadline.
    pub fn purge_date_for_deadline(&self, new_deadline: DateTime<Utc>) -> DateTime<Utc> {
        if new_deadline <= self.purge_date {
            return self.purge_date;
        }
        let gap = self.purge_date - self.voting_deadline;
        new_deadline
            .checked_add_signed(gap)
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    /// Whether the voter must confirm a Response before it is recorded:
    /// only on a Final Ballot, where it can't be changed afterwards.
    pub fn needs_confirmation(&self) -> bool {
        self.submission_mode == SubmissionMode::Final
    }

    /// Whether a selection of `count` distinct Options of this Ballot obeys
    /// its Choice Mode: exactly one Option on a single-choice Ballot, one or
    /// more on a multiple-choice Ballot; never empty. (A Fallback obeys the
    /// same rule.)
    pub fn check_selection_count(&self, count: usize) -> Result<(), &'static str> {
        self.choice_mode.check_selection_count(count)
    }

    /// The Ballot's result at `now`: the voter's Response if there is one;
    /// otherwise, once voting has closed, the Fallback if the admin set one;
    /// otherwise none. Derived, never stored, so extending the deadline
    /// reopens a Ballot naturally.
    pub fn result(&self, now: DateTime<Utc>) -> Option<BallotResult<'_>> {
        match (&self.response, &self.fallback) {
            (Some(response), _) => Some(BallotResult {
                source: ResultSource::Response,
                option_ids: &response.option_ids,
            }),
            (None, Some(fallback)) if !self.is_open(now) => Some(BallotResult {
                source: ResultSource::Fallback,
                option_ids: fallback,
            }),
            (None, _) => None,
        }
    }

    /// The Ballot Status at `now`: Answered with a Response, Auto-fallback
    /// once the Fallback applies, Unanswered otherwise. Derived, never stored.
    pub fn status(&self, now: DateTime<Utc>) -> BallotStatus {
        match self.result(now) {
            Some(BallotResult {
                source: ResultSource::Response,
                ..
            }) => BallotStatus::Answered,
            Some(BallotResult {
                source: ResultSource::Fallback,
                ..
            }) => BallotStatus::AutoFallback,
            None => BallotStatus::Unanswered,
        }
    }
}

/// Why a Ballot accepts no Response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The Ballot is Final and already has a Response.
    AlreadyAnswered,
    /// The Voting Deadline has passed.
    VotingClosed,
    /// The admin edited the Ballot's content since the voter saw it (or
    /// after the voter's selection was checked against it); the voter must
    /// look again.
    BallotChanged,
}

/// What an admin edit touches, by the fields it names (whether or not their
/// values differ from the current ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditScope {
    /// Only the Group and/or the Voter Reference: the admin's bookkeeping,
    /// which the voter never sees. Allowed even once the Ballot is Answered,
    /// and doesn't count as a change of the Ballot for a Response being
    /// submitted at the same time (no new revision).
    Bookkeeping,
    /// Anything else: what the voter sees or what governs their Response.
    Content,
}

/// Why the admin may not edit a Ballot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRefusal {
    /// The Ballot has a Response.
    AlreadyAnswered,
    /// The Voting Deadline has passed without a Response and the Fallback
    /// has applied (Auto-fallback).
    FallbackApplied,
}

/// Where a Ballot stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallotStatus {
    /// No Response yet, or voting closed without a Response and without a
    /// Fallback.
    Unanswered,
    /// The voter submitted a Response.
    Answered,
    /// Voting closed without a Response; the Fallback is the result.
    AutoFallback,
}

impl BallotStatus {
    pub const ALL: [Self; 3] = [Self::Unanswered, Self::Answered, Self::AutoFallback];

    /// The status with this wire name (exact match).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == name)
    }

    /// The wire name, as used by the APIs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unanswered => "unanswered",
            Self::Answered => "answered",
            Self::AutoFallback => "auto_fallback",
        }
    }
}

/// A Ballot's result: the Options that count, and where they come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallotResult<'a> {
    pub source: ResultSource,
    /// IDs of the Options, in the Ballot's Option order.
    pub option_ids: &'a [OptionId],
}

/// Where a Ballot's result comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultSource {
    /// The voter's Response.
    Response,
    /// The admin's Fallback, applied because voting closed without a Response.
    Fallback,
}

impl ResultSource {
    /// The wire name, as used by the APIs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::Fallback => "fallback",
        }
    }
}

/// The Options a voter submitted on a Ballot, with when.
#[derive(Debug, Clone)]
pub struct Response {
    /// IDs of the selected Options, in the Ballot's Option order.
    pub option_ids: Vec<OptionId>,
    pub answered_at: DateTime<Utc>,
    pub changed_at: DateTime<Utc>,
}

/// A valid selection of a Ballot's Options for a Response: only the Ballot's
/// own Options, each at most once, obeying the Ballot's selection rules.
/// Built only by the sanitizers in `crate::input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection(Vec<OptionId>);

impl Selection {
    pub(crate) fn new(option_ids: Vec<OptionId>, _: Sanitized) -> Self {
        Self(option_ids)
    }

    pub fn option_ids(&self) -> &[OptionId] {
        &self.0
    }
}

/// The revision of a Ballot as its voter saw it (on the voting page, its
/// confirmation step, or in the voter API), sent back with a Response: a
/// non-negative integer. A Response is recorded only if it is still the
/// Ballot's revision. Built only by the sanitizers in `crate::input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeenRevision(i64);

impl SeenRevision {
    pub(crate) fn new(revision: i64, _: Sanitized) -> Self {
        Self(revision)
    }

    pub fn get(self) -> i64 {
        self.0
    }

    /// Whether the voter saw `ballot` as it now stands.
    pub fn is_current(self, ballot: &Ballot) -> bool {
        self.0 == ballot.revision
    }
}

/// One Option of a stored Ballot.
#[derive(Debug, Clone)]
pub struct BallotOption {
    pub id: OptionId,
    pub label: String,
}
