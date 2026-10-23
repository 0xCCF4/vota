//! The readable output of `vota ballot`, built from the admin view's
//! JSON. Every text from the server goes through [`printable`], so it can't
//! steer the terminal.

use serde_json::Value;

use super::csv;
use super::{printable, printable_lines};

/// The string at `key`, escaped; `-` when it is `null` or missing.
pub(super) fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => printable(text),
        Some(Value::Null) | None => "-".into(),
        Some(other) => printable(&other.to_string()),
    }
}

/// The labels of `{id, label}` objects, comma-separated; `-` for none.
fn labels(options: Option<&Value>) -> String {
    let labels: Vec<String> = (options.and_then(Value::as_array).into_iter().flatten())
        .map(|option| text(option, "label"))
        .collect();
    if labels.is_empty() {
        "-".into()
    } else {
        labels.join(", ")
    }
}

/// The labels of a Response's or result's Options, like [`labels`], but an
/// empty list (an answer of no Option, on a Ballot that allows it) is
/// `(none)`, told apart from `-` (no answer at all).
fn chosen(options: Option<&Value>) -> String {
    match options.and_then(Value::as_array) {
        Some(options) if options.is_empty() => "(none)".into(),
        _ => labels(options),
    }
}

/// The labels of the Options with these IDs, in the order given, as
/// [`chosen`] shows them.
fn labels_of(ballot: &Value, ids: Option<&Value>) -> String {
    let ids = ids.and_then(Value::as_array).cloned().unwrap_or_default();
    let options: Vec<Value> = (ballot["options"].as_array().into_iter().flatten())
        .filter(|option| ids.contains(&option["id"]))
        .cloned()
        .collect();
    chosen(Some(&Value::Array(options)))
}

/// `yes` or `no` for a boolean at `key`; `-` when it is missing.
fn yes_no(value: &Value, key: &str) -> String {
    match value.get(key).and_then(Value::as_bool) {
        Some(true) => "yes".into(),
        Some(false) => "no".into(),
        None => text(value, key),
    }
}

/// The summary of a new Ballot, for stderr.
pub(super) fn created(ballot: &Value) -> String {
    format!(
        "Created Ballot {}: {}\n{}",
        text(ballot, "id"),
        text(ballot, "title"),
        lines(&[
            ("Status", text(ballot, "status")),
            ("Voting Deadline", text(ballot, "voting_deadline")),
            ("Purge Date", text(ballot, "purge_date")),
        ])
    )
}

/// What `delete` shows before it asks.
pub(super) fn summary(ballot: &Value) -> String {
    format!(
        "Ballot {}: {}\n{}",
        text(ballot, "id"),
        text(ballot, "title"),
        lines(&[
            ("Group", text(ballot, "group")),
            ("Voter Reference", text(ballot, "voter_reference")),
            ("Status", text(ballot, "status")),
        ])
    )
}

/// The admin view of one Ballot as a block of `Name: value` lines.
pub(super) fn ballot(ballot: &Value) -> String {
    let response = match ballot.get("response") {
        Some(response) if response.is_object() => format!(
            "{} (answered {}, changed {})",
            labels_of(ballot, response.get("option_ids")),
            text(response, "answered_at"),
            text(response, "changed_at"),
        ),
        _ => "-".into(),
    };
    let result = match ballot.get("result") {
        Some(result) if result.is_object() => {
            let source = match result["source"].as_str() {
                Some("response") => "from the Response".to_owned(),
                Some("fallback") => "from the Fallback".to_owned(),
                _ => format!("source {}", text(result, "source")),
            };
            format!("{} ({source})", chosen(result.get("options")))
        }
        _ => "-".into(),
    };
    let mut out = format!(
        "Ballot {}: {}\n{}",
        text(ballot, "id"),
        text(ballot, "title"),
        lines(&[
            ("URL", text(ballot, "url")),
            ("Status", text(ballot, "status")),
            ("Group", text(ballot, "group")),
            ("Voter Reference", text(ballot, "voter_reference")),
            ("Choice Mode", text(ballot, "choice_mode")),
            ("Allow No Option", yes_no(ballot, "allow_no_option")),
            ("Submission Mode", text(ballot, "submission_mode")),
            ("Voting Deadline", text(ballot, "voting_deadline")),
            ("Purge Date", text(ballot, "purge_date")),
            ("Created", text(ballot, "created_at")),
            ("Theme", text(ballot, "theme")),
            ("Accent Colour", text(ballot, "accent_color")),
            ("Logo", text(ballot, "logo")),
        ])
    );
    match ballot["description"].as_str() {
        Some(description) => {
            out += "Description:\n";
            for line in description.lines() {
                out += &format!("  {}\n", printable(line));
            }
        }
        None => out += &lines(&[("Description", "-".into())]),
    }
    out += "Options:\n";
    for option in ballot["options"].as_array().into_iter().flatten() {
        out += &format!("  {}  {}\n", text(option, "id"), text(option, "label"));
    }
    out += &lines(&[
        ("Fallback", labels(ballot.get("fallback"))),
        ("Response", response),
        ("Result", result),
    ]);
    out
}

/// `Name: value` lines with the values aligned.
fn lines(rows: &[(&str, String)]) -> String {
    rows.iter()
        .map(|(name, value)| format!("{:<17}{value}\n", format!("{name}:")))
        .collect()
}

/// The list as a table: ID, STATUS, GROUP, VOTER-REFERENCE,
/// VOTING-DEADLINE and RESULT (the result's labels, last because they are
/// free text; `(none)` for an answer of no Option, `-` for no result).
pub(super) fn table(list: &Value) -> String {
    let rows: Vec<[String; 6]> = (list["ballots"].as_array().into_iter().flatten())
        .map(|ballot| {
            [
                text(ballot, "id"),
                text(ballot, "status"),
                text(ballot, "group"),
                text(ballot, "voter_reference"),
                text(ballot, "voting_deadline"),
                match ballot.get("result") {
                    Some(result) if result.is_object() => chosen(result.get("options")),
                    _ => "-".into(),
                },
            ]
        })
        .collect();
    let header = [
        "ID",
        "STATUS",
        "GROUP",
        "VOTER-REFERENCE",
        "VOTING-DEADLINE",
        "RESULT",
    ]
    .map(String::from);
    let mut widths = header.each_ref().map(|cell| cell.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    std::iter::once(&header)
        .chain(&rows)
        .map(|row| {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate().take(5) {
                line += &format!("{cell:<width$}  ", width = widths[i]);
            }
            line += &row[5];
            line.trim_end().to_owned() + "\n"
        })
        .collect()
}

/// The string at `key` for a CSV cell, escaped like [`printable_lines`];
/// empty when it is `null` or missing.
fn csv_text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => printable_lines(text),
        Some(Value::Null) | None => String::new(),
        Some(other) => printable_lines(&other.to_string()),
    }
}

/// The columns of the list's CSV: the admin view's fields of the same name,
/// then the result's labels and source.
const LIST_COLUMNS: [&str; 10] = [
    "id",
    "status",
    "group",
    "voter_reference",
    "voting_deadline",
    "purge_date",
    "choice_mode",
    "submission_mode",
    "result",
    "result_source",
];

/// The list as CSV: one record per Ballot. `result` is the result's labels
/// joined by `; ` (empty without a result, and for an answer of no Option);
/// `result_source` is `answered`, `fallback` or empty. No URLs or Ballot
/// Tokens.
pub(super) fn list_csv(list: &Value) -> String {
    let rows = (list["ballots"].as_array().into_iter().flatten()).map(|ballot| {
        let (labels, source) = match ballot.get("result") {
            Some(result) if result.is_object() => {
                let labels: Vec<String> = (result["options"].as_array().into_iter().flatten())
                    .map(|option| csv_text(option, "label"))
                    .collect();
                let source = match result["source"].as_str() {
                    Some("response") => "answered".to_owned(),
                    _ => csv_text(result, "source"),
                };
                (labels.join("; "), source)
            }
            _ => (String::new(), String::new()),
        };
        let fields = &LIST_COLUMNS[..LIST_COLUMNS.len() - 2];
        let mut row: Vec<String> = fields.iter().map(|key| csv_text(ballot, key)).collect();
        row.extend([labels, source]);
        row
    });
    csv::document(&LIST_COLUMNS, rows)
}

/// The Ballots of a bulk create as a table: VOTER-REFERENCE, ID and URL.
pub(super) fn created_table(answer: &Value) -> String {
    let rows: Vec<[String; 3]> = (answer["ballots"].as_array().into_iter().flatten())
        .map(|ballot| {
            [
                text(ballot, "voter_reference"),
                text(ballot, "id"),
                text(ballot, "url"),
            ]
        })
        .collect();
    let header = ["VOTER-REFERENCE", "ID", "URL"].map(String::from);
    let mut widths = header.each_ref().map(|cell| cell.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    std::iter::once(&header)
        .chain(&rows)
        .map(|row| {
            format!(
                "{:<w0$}  {:<w1$}  {}\n",
                row[0],
                row[1],
                row[2],
                w0 = widths[0],
                w1 = widths[1]
            )
        })
        .collect()
}

/// The summary of a bulk create, for stderr: how many, the shared title,
/// Group and dates.
pub(super) fn created_many(answer: &Value) -> String {
    let ballots = answer["ballots"].as_array().cloned().unwrap_or_default();
    let first = ballots.first().cloned().unwrap_or_default();
    format!(
        "Created {} Ballots: {}\n{}",
        ballots.len(),
        text(&first, "title"),
        lines(&[
            ("Group", text(&first, "group")),
            ("Voting Deadline", text(&first, "voting_deadline")),
            ("Purge Date", text(&first, "purge_date")),
        ])
    )
}

/// The columns of a create's CSV.
const CREATED_COLUMNS: [&str; 3] = ["voter_reference", "id", "url"];

/// The Ballots of a create answer (a bulk answer's `ballots`, or the single
/// Ballot) as CSV: `voter_reference,id,url`, one record per Ballot. Cells
/// are escaped by [`csv_text`] and written by [`csv::document`], like the
/// list's CSV.
pub(super) fn created_csv(answer: &Value) -> String {
    let ballots: Vec<&Value> = match answer.get("ballots").and_then(Value::as_array) {
        Some(ballots) => ballots.iter().collect(),
        None => vec![answer],
    };
    let rows = ballots
        .into_iter()
        .map(|ballot| CREATED_COLUMNS.map(|key| csv_text(ballot, key)));
    csv::document(&CREATED_COLUMNS, rows)
}
