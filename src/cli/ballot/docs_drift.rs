//! The hand-written CLI reference (`docs/reference/cli.md`) against the
//! code: its table of the exit codes of `ballot` against [`Exit`], and its
//! table of the units of relative dates against [`Unit`]. Drift in either
//! direction fails `cargo test`.
//!
//! What is read from the reference, so keep these stable there:
//!
//! - the table right below the heading ``### Exit codes of `ballot` ``: one
//!   row per code, `` | `<code>` | <text> | ``, where the text is
//!   [`Exit::description`] in Markdown (inline code and links allowed), and
//!   names every HTTP status of the code ([`Exit::statuses`]) as `` `NNN` ``;
//! - the table right below the paragraph ending in `The units, in lower
//!   case:`: one row per unit, `` | `<name>` (`<other name>`, …) | <meaning> | ``,
//!   the names of [`Unit::names`] in their order;
//! - the error example of an unreadable date value, which lists
//!   [`unit_list`] verbatim.

use std::collections::BTreeMap;

use super::dates::{Unit, unit_list};
use super::exit::Exit;

fn reference() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/reference/cli.md");
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {path}: {error}"))
}

/// The cells of the table after the first line that ends in `marker`
/// (blank lines and one-line HTML comments between them allowed), without
/// its header and separator rows.
fn table_after(reference: &str, marker: &str) -> Vec<Vec<String>> {
    let mut lines = reference
        .lines()
        .skip_while(|line| !line.trim_end().ends_with(marker));
    assert!(
        lines.next().is_some(),
        "no line ending in {marker:?} in the CLI reference"
    );
    let rows: Vec<Vec<String>> = lines
        .skip_while(|line| line.trim().is_empty() || line.trim().starts_with("<!--"))
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect();
    assert!(rows.len() > 2, "no table below {marker:?}");
    rows.into_iter().skip(2).collect()
}

/// The text of inline Markdown: inline code without its backticks, a link
/// as its text.
fn plain(markdown: &str) -> String {
    let mut text = String::new();
    let mut rest = markdown;
    while let Some(at) = rest.find('[') {
        text += &rest[..at];
        let after = &rest[at + 1..];
        match after
            .find("](")
            .and_then(|close| after[close..].find(')').map(|end| (close, close + end + 1)))
        {
            Some((close, end)) => {
                text += &after[..close];
                rest = &after[end..];
            }
            None => {
                text.push('[');
                rest = after;
            }
        }
    }
    text += rest;
    text.replace('`', "")
}

/// The inline code spans of `markdown`, in order.
fn code_spans(markdown: &str) -> Vec<&str> {
    markdown.split('`').skip(1).step_by(2).collect()
}

#[test]
fn the_exit_codes_of_ballot_in_the_reference_are_those_of_the_code() {
    let reference = reference();
    let rows = table_after(&reference, "### Exit codes of `ballot`");

    let documented: BTreeMap<u8, String> = rows
        .iter()
        .map(|row| {
            assert_eq!(row.len(), 2, "{row:?}");
            let code = code_spans(&row[0]);
            let code = code
                .first()
                .and_then(|code| code.parse().ok())
                .unwrap_or_else(|| panic!("not a code: {:?}", row[0]));
            (code, plain(&row[1]))
        })
        .collect();
    assert_eq!(documented.len(), rows.len(), "a code twice: {rows:?}");
    let code: BTreeMap<u8, String> = Exit::ALL
        .iter()
        .map(|exit| (exit.code(), exit.description().to_owned()))
        .collect();

    assert_eq!(documented, code);
}

#[test]
fn the_reference_names_the_http_statuses_of_each_exit_code() {
    let reference = reference();
    let rows = table_after(&reference, "### Exit codes of `ballot`");

    for row in rows {
        let code: u8 = code_spans(&row[0])[0].parse().unwrap();
        let exit = Exit::ALL
            .into_iter()
            .find(|exit| exit.code() == code)
            .unwrap_or_else(|| panic!("no exit code {code}"));
        let statuses: Vec<u16> = code_spans(&row[1])
            .into_iter()
            .filter(|span| span.len() == 3)
            .filter_map(|span| span.parse().ok())
            .collect();

        assert_eq!(statuses, exit.statuses(), "exit code {code}: {}", row[1]);
    }
}

#[test]
fn the_units_in_the_reference_are_those_of_the_code() {
    let reference = reference();
    let rows = table_after(&reference, "The units, in lower case:");

    let documented: Vec<Vec<&str>> = rows.iter().map(|row| code_spans(&row[0])).collect();
    let code: Vec<Vec<&str>> = Unit::ALL.iter().map(|unit| unit.names().to_vec()).collect();

    assert_eq!(documented, code);
}

#[test]
fn the_reference_quotes_the_unit_list_of_the_error_message() {
    let reference = reference();
    let list = unit_list();

    assert!(
        reference.contains(&format!("without spaces: {list}")),
        "the CLI reference's error example lacks: {list}"
    );
}
