//! Bulk creation: many Ballots from one request, alike but for their Voter
//! References, which come either from a list ([`VoterReferenceList`]) or
//! from numbering on after the Group's highest numeric Voter Reference
//! ([`BallotCount`], [`numbered`]).

use std::collections::HashSet;

use crate::domain::ballot::VoterReference;
use crate::input::Sanitized;

/// At most this many Ballots per request, by list or by count.
pub const MAX_BALLOTS: usize = 1000;

/// How the Ballots of a bulk creation get their Voter References.
#[derive(Debug, Clone)]
pub enum Bulk {
    /// One Ballot per Voter Reference, in the list's order.
    Listed(VoterReferenceList),
    /// This many Ballots with numbered Voter References (requires a Group).
    Numbered(BallotCount),
}

/// Why a list of Voter References is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListRejection {
    Empty,
    TooMany,
    /// The entry at this index repeats an earlier one.
    Duplicate(usize),
}

impl ListRejection {
    /// The fixed reason, never quoting a Voter Reference.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Empty => "at least one Voter Reference is required",
            Self::TooMany => "at most 1000 Voter References per request",
            Self::Duplicate(_) => "duplicate Voter Reference",
        }
    }
}

/// 1..=[`MAX_BALLOTS`] distinct Voter References, in the order given.
#[derive(Debug, Clone)]
pub struct VoterReferenceList(Vec<VoterReference>);

impl VoterReferenceList {
    /// The list, if it is neither empty, too long, nor repeats an entry
    /// (compared as sanitized, so after normalisation).
    pub(crate) fn new(
        references: Vec<VoterReference>,
        _: Sanitized,
    ) -> Result<Self, ListRejection> {
        if references.is_empty() {
            return Err(ListRejection::Empty);
        }
        if references.len() > MAX_BALLOTS {
            return Err(ListRejection::TooMany);
        }
        let mut seen = HashSet::new();
        for (index, reference) in references.iter().enumerate() {
            if !seen.insert(reference.as_str()) {
                return Err(ListRejection::Duplicate(index));
            }
        }
        Ok(Self(references))
    }

    pub fn as_slice(&self) -> &[VoterReference] {
        &self.0
    }
}

/// How many Ballots to create with numbered Voter References:
/// 1..=[`MAX_BALLOTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallotCount(usize);

impl BallotCount {
    /// The count, or the fixed reason it is refused.
    pub fn new(count: i64) -> Result<Self, &'static str> {
        match usize::try_from(count) {
            Ok(count @ 1..=MAX_BALLOTS) => Ok(Self(count)),
            Ok(0) | Err(_) => Err("must be at least 1"),
            Ok(_) => Err("at most 1000 Ballots per request"),
        }
    }

    pub fn get(self) -> usize {
        self.0
    }
}

/// The Group's numeric Voter References leave no room for the count: the
/// numbers would pass `u64::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberingExhausted;

/// The value of a purely numeric Voter Reference: one or more ASCII digits
/// `0`–`9` and nothing else (no sign, space, separator or other script's
/// digits), read as a decimal number, so leading zeros don't count (`007`
/// is 7). `None` for any other Voter Reference; `Some(None)` for a numeric
/// one too large for `u64`.
pub fn numeric_value(reference: &str) -> Option<Option<u64>> {
    if reference.is_empty() || !reference.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(reference.parse().ok())
}

/// `count` Voter References `n+1`, `n+2`, … (plain decimal, no leading
/// zeros), where `n` is the highest [`numeric_value`] among `existing` (the
/// Group's current Voter References), or 0 if there is none. Never clashes
/// with an existing Voter Reference: every numeric one is at most `n`, and
/// the new ones contain only digits.
pub fn numbered<'a>(
    existing: impl IntoIterator<Item = &'a str>,
    count: BallotCount,
) -> Result<Vec<VoterReference>, NumberingExhausted> {
    let mut highest = 0;
    for value in existing.into_iter().filter_map(numeric_value) {
        highest = highest.max(value.ok_or(NumberingExhausted)?);
    }
    let count = u64::try_from(count.get()).map_err(|_| NumberingExhausted)?;
    highest.checked_add(count).ok_or(NumberingExhausted)?;
    Ok((1..=count)
        .map(|offset| VoterReference::numbered(highest + offset))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(n: i64) -> BallotCount {
        BallotCount::new(n).unwrap()
    }

    fn texts(references: &[VoterReference]) -> Vec<&str> {
        references.iter().map(VoterReference::as_str).collect()
    }

    #[test]
    fn numbering_starts_at_one_in_a_group_without_numeric_references() {
        let created = numbered([], count(3)).unwrap();
        assert_eq!(texts(&created), ["1", "2", "3"]);
        let created = numbered(["alice", "bob"], count(2)).unwrap();
        assert_eq!(texts(&created), ["1", "2"]);
    }

    #[test]
    fn numbering_continues_after_the_highest_purely_numeric_reference() {
        let existing = [
            "3",
            "12",
            "007",
            "alice",
            "99a",
            "-50",
            "+40",
            " 30",
            "1e9",
            "٣٣",
            "１００",
            "",
            "2",
        ];
        let created = numbered(existing, count(2)).unwrap();
        assert_eq!(texts(&created), ["13", "14"]);
    }

    #[test]
    fn leading_zeros_are_read_as_the_number() {
        let created = numbered(["0042"], count(1)).unwrap();
        assert_eq!(texts(&created), ["43"]);
    }

    #[test]
    fn numbering_past_u64_is_refused() {
        assert_eq!(
            numbered([u64::MAX.to_string().as_str()], count(1)).unwrap_err(),
            NumberingExhausted
        );
        assert_eq!(
            numbered(["99999999999999999999999"], count(1)).unwrap_err(),
            NumberingExhausted
        );
        let below = (u64::MAX - 1).to_string();
        let created = numbered([below.as_str()], count(1)).unwrap();
        assert_eq!(texts(&created), [u64::MAX.to_string()]);
    }

    #[test]
    fn count_lies_between_one_and_the_maximum() {
        assert!(BallotCount::new(0).is_err());
        assert!(BallotCount::new(-1).is_err());
        assert!(BallotCount::new(1001).is_err());
        assert_eq!(BallotCount::new(1000).unwrap().get(), 1000);
    }
}
