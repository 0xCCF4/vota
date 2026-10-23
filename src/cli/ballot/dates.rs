//! The date flags (`--voting-deadline`, `--purge-date`) of `create`,
//! `edit` and `extend`: a timestamp, sent as given, or a duration, which
//! the client turns into an RFC 3339 UTC timestamp before sending, so the
//! API sees only timestamps:
//!
//! - `3d`: that long from now, by the local clock, to the whole second;
//! - `+7d`: that long after the Voting Deadline, the one given in the same
//!   command, or else the Ballot's current one, which the caller fetches
//!   first ([`Dates::needs_current_deadline`]).
//!
//! A duration is `<number><unit>` groups without spaces ([`Span`]). Its own
//! units, not humantime's as in the server's configuration: `m` is a
//! calendar month here, `min` a minute. Months (and years, as 12 months)
//! are added first, as calendar months (31 January + `1m` is the last day
//! of February), then minutes, hours, days and weeks as fixed spans of
//! 60 s, 1 h, 24 h and 7 × 24 h.
//!
//! A value that starts like a date (`YYYY-`) is a timestamp; the server
//! checks it, and every rule about dates (future, horizon, order). Anything
//! else must be a duration.

use chrono::{DateTime, Months, SecondsFormat, SubsecRound as _, TimeDelta, Utc};

use super::Failure;

/// The command whose date flags are read, with the Ballot's ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Verb {
    Create,
    Edit(u64),
    Extend(u64),
}

/// One date flag as given.
struct Flag {
    /// `--voting-deadline` or `--purge-date`.
    name: &'static str,
    /// The value as given.
    value: String,
    when: When,
}

/// What a date flag says.
enum When {
    /// A timestamp, sent as given.
    At,
    /// This long from now.
    In(Span),
    /// This long after the Voting Deadline (`+…`).
    After(Span),
}

/// A duration as the date flags write it, e.g. `1m2w`: calendar months
/// (`m`, and `y` as 12 months) and a fixed part (`min`, `h`, `d`, `w`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    months: u32,
    fixed: TimeDelta,
}

/// Why a value is not a [`Span`].
enum SpanError {
    /// Not of the form `<number><unit>…` at all.
    Malformed,
    /// A unit that isn't one of [`Unit::names`].
    Unit(String),
    /// More than any date can hold.
    TooLong,
}

impl Span {
    /// `<number><unit>` groups, e.g. `3d`, `1m2w`, `2d12h`, without
    /// spaces; units in lower case.
    fn parse(text: &str) -> Result<Self, SpanError> {
        let mut span = Self {
            months: 0,
            fixed: TimeDelta::zero(),
        };
        let mut rest = text;
        if rest.is_empty() {
            return Err(SpanError::Malformed);
        }
        while !rest.is_empty() {
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits == 0 {
                return Err(SpanError::Malformed);
            }
            let number: u32 = rest[..digits].parse().map_err(|_| SpanError::TooLong)?;
            rest = &rest[digits..];
            let letters = rest.len()
                - rest
                    .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                    .len();
            let unit = &rest[..letters];
            rest = &rest[letters..];
            let fixed = |seconds: i64| {
                TimeDelta::try_seconds(seconds * i64::from(number)).ok_or(SpanError::TooLong)
            };
            if unit.is_empty() {
                return Err(SpanError::Malformed);
            }
            let (months, fixed) = match Unit::parse(unit) {
                Some(Unit::Minute) => (0, fixed(60)?),
                Some(Unit::Hour) => (0, fixed(3600)?),
                Some(Unit::Day) => (0, fixed(86_400)?),
                Some(Unit::Week) => (0, fixed(7 * 86_400)?),
                Some(Unit::Month) => (number, TimeDelta::zero()),
                Some(Unit::Year) => (
                    number.checked_mul(12).ok_or(SpanError::TooLong)?,
                    TimeDelta::zero(),
                ),
                None => return Err(SpanError::Unit(unit.to_owned())),
            };
            span.months = span.months.checked_add(months).ok_or(SpanError::TooLong)?;
            span.fixed = span.fixed.checked_add(&fixed).ok_or(SpanError::TooLong)?;
        }
        Ok(span)
    }

    /// `from` plus the calendar months (a day the target month lacks
    /// becomes its last day), then the fixed part; `None` past any date.
    fn after(self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        from.checked_add_months(Months::new(self.months))?
            .checked_add_signed(self.fixed)
    }
}

/// A unit of a duration: the one place the units are listed, for the
/// parser, the help texts (and so the man pages) and the error message.
/// The CLI reference has the same list; `docs_drift` (a test) compares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    /// A calendar month.
    Month,
    /// 12 calendar months.
    Year,
}

impl Unit {
    /// Every unit, shortest span first.
    pub(super) const ALL: [Self; 6] = [
        Self::Minute,
        Self::Hour,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Year,
    ];

    /// The names a duration may use for the unit, its short name first.
    pub(super) fn names(self) -> &'static [&'static str] {
        match self {
            Self::Minute => &["min", "mins", "minute", "minutes"],
            Self::Hour => &["h"],
            Self::Day => &["d"],
            Self::Week => &["w"],
            Self::Month => &["m", "mo", "month", "months"],
            Self::Year => &["y"],
        }
    }

    /// What the short name means, e.g. `calendar months`.
    fn meaning(self) -> &'static str {
        match self {
            Self::Minute => "minutes",
            Self::Hour => "hours",
            Self::Day => "days",
            Self::Week => "weeks",
            Self::Month => "calendar months",
            Self::Year => "years",
        }
    }

    /// The unit `name` stands for (case matters: `M` is none).
    fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|unit| unit.names().contains(&name))
    }
}

/// The units with their meaning: `min (minutes), h (hours), …`.
pub(super) fn unit_list() -> String {
    Unit::ALL
        .iter()
        .map(|unit| format!("{} ({})", unit.names()[0], unit.meaning()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The warning that `m` is not minutes, unlike in humantime durations;
/// without a final period, like clap's one-paragraph help texts.
pub(super) const M_IS_MONTHS: &str =
    "Note: `m` is months here, unlike in the server's configuration";

/// The end of the help text of a date flag: the units and [`M_IS_MONTHS`].
pub(super) fn units_help() -> String {
    format!(
        "Units, combined without spaces (`2d12h`): {}. {M_IS_MONTHS}",
        unit_list()
    )
}

/// The forms a date flag takes, for its usage error.
fn accepted() -> String {
    format!(
        "give an RFC 3339 timestamp (2026-12-01T12:00:00+01:00), a local date-time in the \
         server's default time zone (2026-12-01T12:00), a duration from now (3d, 1m2w, \
         90min) or + and a duration after the Voting Deadline (+7d); a duration is numbers \
         with units, without spaces: {}",
        unit_list()
    )
}

impl Flag {
    fn parse(name: &'static str, value: &str) -> Result<Self, Failure> {
        let when = if is_timestamp(value) {
            When::At
        } else {
            let (relative, text) = match value.strip_prefix('+') {
                Some(text) => (true, text),
                None => (false, value),
            };
            let delta = Span::parse(text).map_err(|error| {
                let why = match error {
                    SpanError::Malformed => "not a timestamp or a duration".to_owned(),
                    SpanError::Unit(unit) => format!("unknown unit \"{unit}\""),
                    SpanError::TooLong => "the duration is too long for a date".to_owned(),
                };
                Failure::usage(format!("{name} {value}: {why}; {}", accepted()))
            })?;
            if relative {
                When::After(delta)
            } else {
                When::In(delta)
            }
        };
        Ok(Self {
            name,
            value: value.to_owned(),
            when,
        })
    }

    /// `--flag value: why`, a usage error (escaped, like every
    /// [`Failure`]).
    fn refused(&self, why: &str) -> Failure {
        Failure::usage(format!("{} {}: {why}", self.name, self.value))
    }

    /// The instant this flag names: `Ok(None)` for a timestamp that isn't
    /// RFC 3339 (the server reads it, e.g. a local date-time).
    fn instant(
        &self,
        now: DateTime<Utc>,
        deadline: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>, Failure> {
        let (from, delta) = match self.when {
            When::At => {
                return Ok(DateTime::parse_from_rfc3339(&self.value)
                    .ok()
                    .map(|instant| instant.to_utc()));
            }
            When::In(delta) => (now, delta),
            When::After(delta) => (
                deadline.ok_or_else(|| self.refused("no Voting Deadline to be relative to"))?,
                delta,
            ),
        };
        delta
            .after(from)
            .map(Some)
            .ok_or_else(|| self.refused("the duration is too long for a date"))
    }

    /// The value to send: the timestamp as given, or the resolved one,
    /// which gets a line in `notes`.
    fn sent(&self, instant: Option<DateTime<Utc>>, notes: &mut Vec<String>) -> String {
        let (Some(instant), When::In(_) | When::After(_)) = (instant, &self.when) else {
            return self.value.clone();
        };
        let sent = instant.to_rfc3339_opts(SecondsFormat::AutoSi, true);
        let (field, how) = if self.name == "--voting-deadline" {
            ("Voting Deadline", "after the current Voting Deadline")
        } else {
            ("Purge Date", "after the Voting Deadline")
        };
        // Only a duration gets here, which `Span::parse` read: ASCII
        // digits and unit letters, nothing to escape.
        let relation = match self.when {
            When::After(_) => format!("{} {how}", &self.value[1..]),
            _ => format!("in {}", self.value),
        };
        notes.push(format!("{field}: {sent} ({relation})"));
        sent
    }
}

/// Whether `value` starts like a date (`YYYY-`): a timestamp for the server.
fn is_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() > 4 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-'
}

/// The date flags of one command, parsed.
pub(super) struct Dates {
    voting_deadline: Option<Flag>,
    purge_date: Option<Flag>,
}

/// The date fields to send.
pub(super) struct Resolved {
    pub(super) voting_deadline: Option<String>,
    pub(super) purge_date: Option<String>,
    /// One line per resolved relative value, for stderr, e.g.
    /// `Voting Deadline: 2026-10-25T14:00:00Z (in 3d)`.
    pub(super) notes: Vec<String>,
}

impl Dates {
    /// Parses the flags; a `+` that can't be relative to anything (on
    /// `create`) is a usage error.
    pub(super) fn parse(
        voting_deadline: Option<&str>,
        purge_date: Option<&str>,
        verb: Verb,
    ) -> Result<Self, Failure> {
        let voting_deadline = voting_deadline
            .map(|value| Flag::parse("--voting-deadline", value))
            .transpose()?;
        let purge_date = purge_date
            .map(|value| Flag::parse("--purge-date", value))
            .transpose()?;
        if verb == Verb::Create {
            if let Some(flag) = &voting_deadline
                && matches!(flag.when, When::After(_))
            {
                return Err(flag.refused(
                    "+ is relative to the current Voting Deadline, and a new Ballot has none; \
                     give a timestamp or a duration from now (3d)",
                ));
            }
            if let Some(flag) = &purge_date
                && matches!(flag.when, When::After(_))
                && voting_deadline.is_none()
            {
                return Err(flag.refused(
                    "+ is relative to the Voting Deadline, and a new Ballot has none yet; \
                     give --voting-deadline in the same command",
                ));
            }
        }
        Ok(Self {
            voting_deadline,
            purge_date,
        })
    }

    /// Whether a `+` value is relative to the Ballot's current Voting
    /// Deadline, which must then be fetched for [`Self::resolve`].
    pub(super) fn needs_current_deadline(&self) -> bool {
        let after = |flag: &Option<Flag>| {
            matches!(
                flag,
                Some(Flag {
                    when: When::After(_),
                    ..
                })
            )
        };
        after(&self.voting_deadline) || (self.voting_deadline.is_none() && after(&self.purge_date))
    }

    /// The values to send, with `now` the local clock's time and `current`
    /// the Ballot's current Voting Deadline, if fetched.
    pub(super) fn resolve(
        self,
        now: DateTime<Utc>,
        current: Option<DateTime<Utc>>,
    ) -> Result<Resolved, Failure> {
        let now = now.trunc_subsecs(0);
        let mut deadline = current;
        let mut resolved = Resolved {
            voting_deadline: None,
            purge_date: None,
            notes: Vec::new(),
        };
        if let Some(flag) = &self.voting_deadline {
            let instant = flag.instant(now, current)?;
            deadline = instant;
            resolved.voting_deadline = Some(flag.sent(instant, &mut resolved.notes));
        }
        if let Some(flag) = &self.purge_date {
            if matches!(flag.when, When::After(_))
                && deadline.is_none()
                && let Some(given) = &self.voting_deadline
            {
                return Err(flag.refused(&format!(
                    "+ is relative to {} {}, which the client can't read: it is not \
                     RFC 3339 with an offset (a local time is read by the server); give \
                     the Voting Deadline with an offset or as a duration, or the Purge \
                     Date as a timestamp",
                    given.name, given.value
                )));
            }
            let instant = flag.instant(now, deadline)?;
            resolved.purge_date = Some(flag.sent(instant, &mut resolved.notes));
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ballot::exit::Exit;
    use chrono::{DateTime, Utc};

    fn now() -> DateTime<Utc> {
        "2026-10-22T14:00:00.750Z".parse().unwrap()
    }

    #[test]
    fn a_bare_duration_is_from_now_to_the_whole_second() {
        let dates = Dates::parse(Some("3d"), None, Verb::Create).ok().unwrap();

        let resolved = dates.resolve(now(), None).ok().unwrap();

        assert_eq!(
            resolved.voting_deadline.as_deref(),
            Some("2026-10-25T14:00:00Z")
        );
        assert_eq!(resolved.purge_date, None);
    }

    #[test]
    fn an_absolute_timestamp_is_sent_unchanged_even_when_the_server_will_refuse_it() {
        for value in [
            "2026-12-01T12:00:00+01:00",
            "2026-12-01T12:00",
            "2026-13-45T99:99",
        ] {
            let dates = Dates::parse(Some(value), Some(value), Verb::Create)
                .ok()
                .unwrap();

            let resolved = dates.resolve(now(), None).ok().unwrap();

            assert_eq!(resolved.voting_deadline.as_deref(), Some(value));
            assert_eq!(resolved.purge_date.as_deref(), Some(value));
        }
    }

    #[test]
    fn a_plus_purge_date_is_after_the_deadline_given_in_the_same_command() {
        for (deadline, purge_date) in [
            ("3d", "2026-11-01T14:00:00Z"),
            ("2026-12-01T12:00:00+01:00", "2026-12-08T11:00:00Z"),
            ("2026-12-01T12:00:00.250Z", "2026-12-08T12:00:00.250Z"),
        ] {
            let dates = Dates::parse(Some(deadline), Some("+7d"), Verb::Create)
                .ok()
                .unwrap();

            let resolved = dates.resolve(now(), None).ok().unwrap();

            assert_eq!(
                resolved.purge_date.as_deref(),
                Some(purge_date),
                "{deadline}"
            );
        }
    }

    #[test]
    fn plus_on_edit_and_extend_is_after_the_ballots_current_deadline() {
        let current: DateTime<Utc> = "2026-11-01T09:30:00.125Z".parse().unwrap();
        for (verb, deadline, purge_date, sent_deadline, sent_purge_date) in [
            (
                Verb::Edit(1),
                None,
                Some("+2h"),
                None,
                Some("2026-11-01T11:30:00.125Z"),
            ),
            (
                Verb::Edit(1),
                Some("+1d"),
                Some("+7d"),
                Some("2026-11-02T09:30:00.125Z"),
                Some("2026-11-09T09:30:00.125Z"),
            ),
            (
                Verb::Extend(1),
                Some("+1d"),
                None,
                Some("2026-11-02T09:30:00.125Z"),
                None,
            ),
        ] {
            let dates = Dates::parse(deadline, purge_date, verb).ok().unwrap();
            assert!(dates.needs_current_deadline());

            let resolved = dates.resolve(now(), Some(current)).ok().unwrap();

            assert_eq!(resolved.voting_deadline.as_deref(), sent_deadline);
            assert_eq!(resolved.purge_date.as_deref(), sent_purge_date);
        }
    }

    #[test]
    fn the_current_deadline_is_needed_only_for_plus_without_a_deadline_to_follow() {
        for (deadline, purge_date) in [
            (None, None),
            (Some("3d"), Some("+7d")),
            (Some("2026-12-01T12:00:00Z"), Some("+7d")),
            (None, Some("30d")),
        ] {
            let dates = Dates::parse(deadline, purge_date, Verb::Edit(1))
                .ok()
                .unwrap();

            assert!(
                !dates.needs_current_deadline(),
                "{deadline:?} {purge_date:?}"
            );
        }
    }

    /// The usage error of `parse` or `resolve`.
    fn refused(result: Result<Resolved, Failure>) -> String {
        let Err(failure) = result else {
            panic!("accepted");
        };
        assert_eq!(failure.exit, Exit::Usage, "{}", failure.message);
        failure.message.to_string()
    }

    #[test]
    fn a_value_that_is_neither_a_timestamp_nor_a_duration_is_a_usage_error_naming_the_forms() {
        for value in [
            "next friday",
            "",
            "+",
            "-1d",
            "+next",
            "3 days ago",
            "1d+",
            "3x",
        ] {
            for (deadline, purge_date, flag) in [
                (Some(value), None, "--voting-deadline"),
                (None, Some(value), "--purge-date"),
            ] {
                let message = refused(
                    Dates::parse(deadline, purge_date, Verb::Edit(1))
                        .and_then(|dates| dates.resolve(now(), Some(now()))),
                );

                assert!(
                    message.starts_with(&format!("{flag} {value}: ")),
                    "{message}"
                );
                for form in ["RFC 3339", "2026-12-01T12:00", "3d", "+"] {
                    assert!(message.contains(form), "{form} in {message}");
                }
            }
        }
    }

    #[test]
    fn plus_needs_a_deadline_to_be_relative_to() {
        for (deadline, purge_date, flag) in [
            (Some("+1d"), None, "--voting-deadline +1d"),
            (None, Some("+7d"), "--purge-date +7d"),
            (Some("+1d"), Some("+7d"), "--voting-deadline +1d"),
        ] {
            let message = refused(
                Dates::parse(deadline, purge_date, Verb::Create)
                    .and_then(|dates| dates.resolve(now(), None)),
            );

            assert!(message.starts_with(&format!("{flag}: ")), "{message}");
            assert!(message.contains("new Ballot"), "{message}");
        }
    }

    #[test]
    fn a_plus_purge_date_needs_a_deadline_with_an_offset() {
        let message = refused(
            Dates::parse(Some("2026-12-01T12:00"), Some("+7d"), Verb::Edit(1))
                .and_then(|dates| dates.resolve(now(), None)),
        );

        assert!(message.starts_with("--purge-date +7d: "), "{message}");
        assert!(message.contains("with an offset"), "{message}");
    }

    #[test]
    fn a_duration_beyond_any_date_is_a_usage_error() {
        let message = refused(
            Dates::parse(Some("100000000000y"), None, Verb::Create)
                .and_then(|dates| dates.resolve(now(), None)),
        );

        assert!(
            message.starts_with("--voting-deadline 100000000000y: "),
            "{message}"
        );
    }

    /// The deadline sent for `--voting-deadline value` with `now()`.
    fn from_now(value: &str) -> String {
        let dates = Dates::parse(Some(value), None, Verb::Create).ok().unwrap();
        dates
            .resolve(now(), None)
            .ok()
            .unwrap()
            .voting_deadline
            .unwrap()
    }

    #[test]
    fn m_is_months_and_min_is_minutes() {
        for (value, sent) in [
            ("1m", "2026-11-22T14:00:00Z"),
            ("1mo", "2026-11-22T14:00:00Z"),
            ("2months", "2026-12-22T14:00:00Z"),
            ("1month", "2026-11-22T14:00:00Z"),
            ("10min", "2026-10-22T14:10:00Z"),
            ("1mins", "2026-10-22T14:01:00Z"),
            ("1minute", "2026-10-22T14:01:00Z"),
            ("90minutes", "2026-10-22T15:30:00Z"),
            ("2h", "2026-10-22T16:00:00Z"),
            ("1w", "2026-10-29T14:00:00Z"),
            ("1y", "2027-10-22T14:00:00Z"),
        ] {
            assert_eq!(from_now(value), sent, "{value}");
        }
    }

    #[test]
    fn units_combine_months_and_years_first_then_the_rest() {
        for (value, sent) in [
            ("1m2w", "2026-12-06T14:00:00Z"),
            ("2d12h", "2026-10-25T02:00:00Z"),
            ("1y1m1w1d1h1min", "2027-11-30T15:01:00Z"),
            ("1w1m", "2026-11-29T14:00:00Z"),
        ] {
            assert_eq!(from_now(value), sent, "{value}");
        }
    }

    #[test]
    fn a_month_from_a_day_the_next_month_lacks_is_its_last_day() {
        let current: DateTime<Utc> = "2027-01-31T12:00:00Z".parse().unwrap();
        for (value, sent) in [
            ("+1m", "2027-02-28T12:00:00Z"),
            ("+1m1d", "2027-03-01T12:00:00Z"),
            ("+3m", "2027-04-30T12:00:00Z"),
        ] {
            let dates = Dates::parse(Some(value), None, Verb::Extend(1))
                .ok()
                .unwrap();

            let resolved = dates.resolve(now(), Some(current)).ok().unwrap();

            assert_eq!(resolved.voting_deadline.as_deref(), Some(sent), "{value}");
        }
    }

    #[test]
    fn an_unknown_unit_is_refused_with_the_units_listed() {
        for value in ["3M", "2s", "1x", "+5hours", "3 d"] {
            let message = refused(
                Dates::parse(Some(value), None, Verb::Edit(1))
                    .and_then(|dates| dates.resolve(now(), Some(now()))),
            );

            assert!(message.contains("min (minutes)"), "{message}");
            assert!(message.contains("m (calendar months)"), "{message}");
        }
    }

    #[test]
    fn each_resolved_value_gets_a_note_and_a_timestamp_none() {
        let dates = Dates::parse(Some("3d"), Some("+1m"), Verb::Create)
            .ok()
            .unwrap();

        let resolved = dates.resolve(now(), None).ok().unwrap();

        assert_eq!(
            resolved.notes,
            [
                "Voting Deadline: 2026-10-25T14:00:00Z (in 3d)",
                "Purge Date: 2026-11-25T14:00:00Z (1m after the Voting Deadline)",
            ]
        );

        let dates = Dates::parse(Some("2026-12-01T12:00:00Z"), Some("+7d"), Verb::Create)
            .ok()
            .unwrap();

        let resolved = dates.resolve(now(), None).ok().unwrap();

        assert_eq!(
            resolved.notes,
            ["Purge Date: 2026-12-08T12:00:00Z (7d after the Voting Deadline)"]
        );
    }
}
