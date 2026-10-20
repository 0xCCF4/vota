//! The readable output of `vota ballot`, built from the admin view's
//! JSON. Every text from the server goes through [`printable`], so it can't
//! steer the terminal.

use serde_json::Value;

use super::printable;

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

/// The labels of the Options with these IDs, in the order given.
fn labels_of(ballot: &Value, ids: Option<&Value>) -> String {
    let ids = ids.and_then(Value::as_array).cloned().unwrap_or_default();
    let options: Vec<Value> = (ballot["options"].as_array().into_iter().flatten())
        .filter(|option| ids.contains(&option["id"]))
        .cloned()
        .collect();
    labels(Some(&Value::Array(options)))
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
            format!("{} ({source})", labels(result.get("options")))
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
/// free text).
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
                    Some(result) if result.is_object() => labels(result.get("options")),
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
