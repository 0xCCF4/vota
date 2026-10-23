//! `vota ballot tally`: the results of a list of Ballots (`GET
//! /admin/ballots`) summed up by Option label, in the client. The same
//! label on two Ballots is the same Option.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::exit::Exit;
use super::{Failure, csv, printable, printable_lines};

/// What a list of Ballots adds up to.
pub(super) struct Tally {
    /// The `--group` filter; `None` for all live Ballots.
    group: Option<String>,
    ballots: u64,
    /// Ballots whose result is the voter's Response.
    answered: u64,
    /// Ballots whose result is the Fallback.
    fallback: u64,
    /// Ballots without a result yet.
    open: u64,
    /// One row per Option label, sorted; then the row of a Response of no
    /// Option, if any Ballot allows one.
    rows: Vec<Row>,
    /// Whether any Ballot is multiple choice (its totals may exceed the
    /// number of Ballots).
    multiple: bool,
}

/// The counts of one Option label; `label` is `None` for a Response of no
/// Option.
#[derive(Default)]
struct Row {
    label: Option<String>,
    answered: u64,
    fallback: u64,
}

impl Row {
    /// A row of this label with no counts yet.
    fn of(label: String) -> Self {
        Self {
            label: Some(label),
            ..Self::default()
        }
    }

    fn total(&self) -> u64 {
        self.answered + self.fallback
    }
}

impl Tally {
    /// Sums up the API's `{"ballots": [...]}`. Every Option of every
    /// Ballot gets a row, also one nobody chose. A result's `source` other
    /// than `response` or `fallback` is an unexpected answer.
    pub(super) fn of(list: &Value, group: Option<String>) -> Result<Self, Failure> {
        let mut tally = Self {
            group,
            ballots: 0,
            answered: 0,
            fallback: 0,
            open: 0,
            rows: Vec::new(),
            multiple: false,
        };
        let mut labels: BTreeMap<String, Row> = BTreeMap::new();
        let mut none = Row::default();
        let mut allows_none = false;
        let label = |option: &Value| option["label"].as_str().unwrap_or_default().to_owned();
        for ballot in list["ballots"].as_array().into_iter().flatten() {
            tally.ballots += 1;
            tally.multiple |= ballot["choice_mode"] == "multiple";
            allows_none |= ballot["allow_no_option"] == true;
            for option in ballot["options"].as_array().into_iter().flatten() {
                let label = label(option);
                labels
                    .entry(label.clone())
                    .or_insert_with(|| Row::of(label));
            }
            let result = &ballot["result"];
            if result.is_null() {
                tally.open += 1;
                continue;
            }
            let from_response = match result["source"].as_str() {
                Some("response") => true,
                Some("fallback") => false,
                _ => {
                    return Err(Failure::new(
                        Exit::Other,
                        format!(
                            "unexpected response: Ballot {} with a result of unknown source",
                            printable(&ballot["id"].to_string())
                        ),
                    ));
                }
            };
            let count = |row: &mut Row| {
                if from_response {
                    row.answered += 1;
                } else {
                    row.fallback += 1;
                }
            };
            if from_response {
                tally.answered += 1;
            } else {
                tally.fallback += 1;
            }
            let chosen = result["options"].as_array().cloned().unwrap_or_default();
            if chosen.is_empty() {
                count(&mut none);
            }
            for option in &chosen {
                let label = label(option);
                count(
                    labels
                        .entry(label.clone())
                        .or_insert_with(|| Row::of(label)),
                );
            }
        }
        // By label first (the map's order), then by total: a stable sort
        // keeps the label order among equal totals.
        tally.rows = labels.into_values().collect();
        tally.rows.sort_by_key(|row| Reverse(row.total()));
        if allows_none || none.total() > 0 {
            tally.rows.push(none);
        }
        Ok(tally)
    }

    /// The readable output: a header line, the table, and for multiple
    /// choice a note below it.
    pub(super) fn text(&self) -> String {
        let group = match &self.group {
            Some(group) => printable(group),
            None => "(all live Ballots)".into(),
        };
        let mut out = format!(
            "Group: {group}  Ballots: {}  Answered: {}  By Fallback: {}  Open: {}\n\n",
            self.ballots, self.answered, self.fallback, self.open
        );
        let rows: Vec<[String; 4]> = self
            .rows
            .iter()
            .map(|row| {
                [
                    row.label.as_deref().map_or("(none)".into(), printable),
                    row.answered.to_string(),
                    row.fallback.to_string(),
                    row.total().to_string(),
                ]
            })
            .collect();
        let header = ["OPTION", "ANSWERED", "FALLBACK", "TOTAL"].map(String::from);
        let mut widths = header.each_ref().map(|cell| cell.chars().count());
        for row in &rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count());
            }
        }
        for row in std::iter::once(&header).chain(&rows) {
            // Labels left-aligned, counts right-aligned.
            let mut line = format!("{:<width$}", row[0], width = widths[0]);
            for (cell, width) in row.iter().zip(widths).skip(1) {
                line += &format!("  {cell:>width$}");
            }
            out += line.trim_end();
            out += "\n";
        }
        if self.multiple {
            out += "\nNote: on multiple-choice Ballots each chosen Option counts once, \
                    so the totals can exceed the number of Ballots.\n";
        }
        out
    }

    /// The JSON output: `{group, ballots, answered, fallback, open,
    /// options: [{option, answered, fallback, total}]}`, `option` `null`
    /// for a Response of no Option.
    pub(super) fn json(&self) -> Value {
        let options: Vec<Value> = self
            .rows
            .iter()
            .map(|row| {
                json!({
                    "option": row.label,
                    "answered": row.answered,
                    "fallback": row.fallback,
                    "total": row.total(),
                })
            })
            .collect();
        json!({
            "group": self.group,
            "ballots": self.ballots,
            "answered": self.answered,
            "fallback": self.fallback,
            "open": self.open,
            "options": options,
        })
    }

    /// The CSV output: `option,answered,fallback,total`, the `option` cell
    /// empty for a Response of no Option (no Option has an empty label).
    pub(super) fn csv(&self) -> String {
        csv::document(
            &["option", "answered", "fallback", "total"],
            self.rows.iter().map(|row| {
                [
                    row.label
                        .as_deref()
                        .map(printable_lines)
                        .unwrap_or_default(),
                    row.answered.to_string(),
                    row.fallback.to_string(),
                    row.total().to_string(),
                ]
            }),
        )
    }
}
