//! The readable output of `vota ballot`, built from the views of
//! [`answer`](super::answer), whose text is escaped already: it can't steer
//! the terminal.

use super::answer::{Created, Origin, ServerBallot, ServerOption, ServerText, Shown, Source};
use super::{PrintableLines, PrintableText, csv};

/// The labels of `options`, comma-separated; `-` for none.
fn labels<'a>(options: impl IntoIterator<Item = &'a ServerOption>) -> String {
    let labels: Vec<String> = (options.into_iter())
        .map(|option| option.label.show().to_string())
        .collect();
    if labels.is_empty() {
        "-".into()
    } else {
        labels.join(", ")
    }
}

/// The labels of a Response's or result's Options, like [`labels`], but an
/// empty list (an answer of no Option, on a Ballot that allows it) is
/// `(none)`, told apart from `-` (no answer at all, or no list).
fn chosen(options: Option<&[ServerOption]>) -> String {
    match options {
        Some([]) => "(none)".into(),
        Some(options) => labels(options),
        None => "-".into(),
    }
}

/// The labels of the Ballot's Options with these IDs, as [`chosen`] shows
/// them.
fn labels_of(ballot: &ServerBallot, ids: &[Shown<ServerText>]) -> String {
    let options: Vec<ServerOption> = (ballot.options.iter())
        .filter(|option| ids.contains(&option.id))
        .cloned()
        .collect();
    chosen(Some(&options))
}

/// The summary of a new Ballot, for stderr.
pub(super) fn created(ballot: &ServerBallot) -> String {
    format!(
        "Created Ballot {}: {}\n{}",
        ballot.id.show(),
        ballot.title.show(),
        lines(&[
            ("Status", ballot.status.show()),
            ("Voting Deadline", ballot.voting_deadline.show()),
            ("Purge Date", ballot.purge_date.show()),
        ])
    )
}

/// What `delete` shows before it asks.
pub(super) fn summary(ballot: &ServerBallot) -> String {
    format!(
        "Ballot {}: {}\n{}",
        ballot.id.show(),
        ballot.title.show(),
        lines(&[
            ("Group", ballot.group.show()),
            ("Voter Reference", ballot.voter_reference.show()),
            ("Status", ballot.status.show()),
        ])
    )
}

/// The admin view of one Ballot as a block of `Name: value` lines.
pub(super) fn ballot(ballot: &ServerBallot) -> String {
    let response = match &ballot.response {
        Some(response) if response.origin == Shown::Known(Origin::Preselection) => format!(
            "{} (preselected {}, not saved by the voter)",
            labels_of(ballot, &response.option_ids),
            response.answered_at.show(),
        ),
        Some(response) => format!(
            "{} (answered {}, changed {})",
            labels_of(ballot, &response.option_ids),
            response.answered_at.show(),
            response.changed_at.show(),
        ),
        None => "-".into(),
    };
    let result = match &ballot.result {
        Some(result) => {
            let source = match result.source {
                Shown::Known(Source::Response) => "from the Response".to_owned(),
                Shown::Known(Source::Preselection) => "from the Preselection".to_owned(),
                Shown::Known(Source::Fallback) => "from the Fallback".to_owned(),
                _ => format!("source {}", result.source.show()),
            };
            format!("{} ({source})", chosen(result.options.as_deref()))
        }
        None => "-".into(),
    };
    let mut out = format!(
        "Ballot {}: {}\n{}",
        ballot.id.show(),
        ballot.title.show(),
        lines(&[
            ("URL", ballot.url.show()),
            ("Status", ballot.status.show()),
            ("Group", ballot.group.show()),
            ("Voter Reference", ballot.voter_reference.show()),
            ("Choice Mode", ballot.choice_mode.show()),
            ("Allow No Option", ballot.allow_no_option.show()),
            ("Submission Mode", ballot.submission_mode.show()),
            ("Voting Deadline", ballot.voting_deadline.show()),
            ("Purge Date", ballot.purge_date.show()),
            ("Created", ballot.created_at.show()),
            ("Theme", ballot.theme.show()),
            ("Accent Colour", ballot.accent_color.show()),
            ("Logo", ballot.logo.show()),
        ])
    );
    match ballot.description.known() {
        Some(description) => {
            out += "Description:\n";
            for line in description.each_line() {
                out += &format!("  {line}\n");
            }
        }
        None => out += &lines(&[("Description", PrintableText::new("-"))]),
    }
    out += "Options:\n";
    for option in &ballot.options {
        out += &format!("  {}  {}\n", option.id.show(), option.label.show());
    }
    out += &lines(&[
        ("Fallback", PrintableText::new(&labels(&ballot.fallback))),
        (
            "Preselection",
            PrintableText::new(&labels(&ballot.preselection)),
        ),
        ("Response", PrintableText::new(&response)),
        ("Result", PrintableText::new(&result)),
    ]);
    out
}

/// `Name: value` lines with the values aligned.
fn lines(rows: &[(&str, PrintableText)]) -> String {
    rows.iter()
        .map(|(name, value)| format!("{:<17}{value}\n", format!("{name}:")))
        .collect()
}

/// The list as a table: ID, STATUS, GROUP, VOTER-REFERENCE,
/// VOTING-DEADLINE and RESULT (the result's labels, last because they are
/// free text; `(none)` for an answer of no Option, `-` for no result).
pub(super) fn table(ballots: &[ServerBallot]) -> String {
    let rows: Vec<[String; 6]> = ballots
        .iter()
        .map(|ballot| {
            [
                ballot.id.show().to_string(),
                ballot.status.show().to_string(),
                ballot.group.show().to_string(),
                ballot.voter_reference.show().to_string(),
                ballot.voting_deadline.show().to_string(),
                match &ballot.result {
                    Some(result) => chosen(result.options.as_deref()),
                    None => "-".into(),
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
/// `result_source` is `answered` (the voter's Response), `preselected` (the
/// Preselection, recorded as the Response of an Editable Ballot the voter
/// hasn't saved), `fallback` or empty. No URLs or Ballot Tokens.
pub(super) fn list_csv(ballots: &[ServerBallot]) -> String {
    let rows = ballots.iter().map(|ballot| {
        let (labels, source) = match &ballot.result {
            Some(result) => {
                let labels: Vec<String> = (result.options.iter().flatten())
                    .map(|option| option.label.cell().as_str().to_owned())
                    .collect();
                let source = match result.source {
                    Shown::Known(Source::Response) => PrintableLines::new("answered"),
                    Shown::Known(Source::Preselection) => PrintableLines::new("preselected"),
                    _ => result.source.cell(),
                };
                (PrintableLines::new(&labels.join("; ")), source)
            }
            None => (PrintableLines::default(), PrintableLines::default()),
        };
        vec![
            ballot.id.cell(),
            ballot.status.cell(),
            ballot.group.cell(),
            ballot.voter_reference.cell(),
            ballot.voting_deadline.cell(),
            ballot.purge_date.cell(),
            ballot.choice_mode.cell(),
            ballot.submission_mode.cell(),
            labels,
            source,
        ]
    });
    csv::document(&LIST_COLUMNS, rows)
}

/// The Ballots of a bulk create as a table: VOTER-REFERENCE, ID and URL.
pub(super) fn created_table(ballots: &[ServerBallot]) -> String {
    let rows: Vec<[String; 3]> = ballots
        .iter()
        .map(|ballot| {
            [
                ballot.voter_reference.show().to_string(),
                ballot.id.show().to_string(),
                ballot.url.show().to_string(),
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
pub(super) fn created_many(ballots: &[ServerBallot]) -> String {
    let none = ServerBallot::default();
    let first = ballots.first().unwrap_or(&none);
    format!(
        "Created {} Ballots: {}\n{}",
        ballots.len(),
        first.title.show(),
        lines(&[
            ("Group", first.group.show()),
            ("Voting Deadline", first.voting_deadline.show()),
            ("Purge Date", first.purge_date.show()),
        ])
    )
}

/// The columns of a create's CSV.
const CREATED_COLUMNS: [&str; 3] = ["voter_reference", "id", "url"];

/// The Ballots of a create answer (many, or the single Ballot) as CSV:
/// `voter_reference,id,url`, one record per Ballot, written by
/// [`csv::document`] like the list's CSV.
pub(super) fn created_csv(created: &Created) -> String {
    let ballots = match created {
        Created::One(ballot) => std::slice::from_ref(ballot.as_ref()),
        Created::Many(ballots) => ballots,
    };
    let rows = ballots.iter().map(|ballot| {
        [
            ballot.voter_reference.cell(),
            ballot.id.cell(),
            ballot.url.cell(),
        ]
    });
    csv::document(&CREATED_COLUMNS, rows)
}
