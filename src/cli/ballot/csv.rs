//! CSV output of `vota ballot`: RFC 4180, UTF-8 (no byte order mark), a
//! header row, each record ended by CRLF (as the RFC says; line breaks
//! inside a quoted cell are kept as they are). Quoting only where needed,
//! and a cell that a spreadsheet would run as a formula is kept text with
//! a leading `'`.
//!
//! A thin wrapper around the `csv` crate, which does the quoting; the
//! formula protection is ours, applied to each cell before it is written.
//! The cells arrive escaped already, as [`PrintableLines`] (`cli/escape.rs`),
//! so nothing in them can steer a terminal; line feeds and tabs are kept.

use std::borrow::Cow;

use csv::{QuoteStyle, Terminator, Writer, WriterBuilder};

use super::PrintableLines;

/// A whole CSV document: the `header` row (the client's own names), then
/// each of `rows`.
pub(super) fn document<R>(header: &[&str], rows: impl IntoIterator<Item = R>) -> String
where
    R: IntoIterator<Item = PrintableLines>,
{
    let mut writer = writer();
    write_record(&mut writer, header);
    for row in rows {
        write_record(
            &mut writer,
            row.into_iter().map(|cell| cell.as_str().to_owned()),
        );
    }
    finish(writer)
}

/// One record: the cells, each as [`cell`] writes it, joined by `,` and
/// ended by CRLF.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "shared API of the CSV writer, used by its tests; the CLI writes whole documents"
    )
)]
pub(super) fn record<C: AsRef<str>>(cells: impl IntoIterator<Item = C>) -> String {
    let mut writer = writer();
    write_record(&mut writer, cells);
    finish(writer)
}

/// One cell as written in a record. A value a spreadsheet would read as a
/// formula (one starting with `=`, `+`, `-`, `@`, tab or CR) gets a
/// leading `'`, so it stays text; then a value holding `,`, `"`, CR or LF
/// is put in double quotes, with each `"` doubled.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "shared API of the CSV writer, used by its tests; the CLI writes whole documents"
    )
)]
pub(super) fn cell(value: &str) -> String {
    // The `csv` crate writes a record of one empty cell as `""`, to tell it
    // from an empty record; as one cell among others it is empty.
    if value.is_empty() {
        return String::new();
    }
    let mut text = record([value]);
    text.truncate(text.len() - "\r\n".len());
    text
}

/// A writer into memory: `,` between cells, CRLF after each record,
/// quotes only where needed (`,`, `"`, CR or LF in the cell), `"` doubled.
/// Flexible, so records of any length are written as given.
fn writer() -> Writer<Vec<u8>> {
    WriterBuilder::new()
        .terminator(Terminator::CRLF)
        .quote_style(QuoteStyle::Necessary)
        .double_quote(true)
        .flexible(true)
        .from_writer(Vec::new())
}

/// Writes one record of protected cells.
fn write_record<C: AsRef<str>>(writer: &mut Writer<Vec<u8>>, cells: impl IntoIterator<Item = C>) {
    let cells: Vec<String> = cells
        .into_iter()
        .map(|cell| protect(cell.as_ref()).into_owned())
        .collect();
    // The only errors are I/O errors of the underlying writer, and a
    // `Vec<u8>` never fails; a record of another length isn't one, since
    // the writer is flexible.
    writer
        .write_record(&cells)
        .expect("writing CSV into memory can't fail");
}

/// The text written so far.
fn finish(writer: Writer<Vec<u8>>) -> String {
    // Flushing into a `Vec<u8>` can't fail.
    let bytes = writer
        .into_inner()
        .expect("flushing CSV into memory can't fail");
    // Only `str` cells and ASCII quotes, commas and line ends went in.
    String::from_utf8(bytes).expect("CSV of UTF-8 cells is UTF-8")
}

/// `value` with a leading `'` when a spreadsheet would read it as a
/// formula: when it starts with `=`, `+`, `-`, `@`, tab or CR.
fn protect(value: &str) -> Cow<'_, str> {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        Cow::Owned(format!("'{value}"))
    } else {
        Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_a_header_and_records_ended_by_crlf() {
        let rows = [["1", "Pizza"], ["2", "Sushi"]].map(|row| row.map(PrintableLines::new));

        let text = document(&["id", "name"], rows);

        assert_eq!(text, "id,name\r\n1,Pizza\r\n2,Sushi\r\n");
    }

    #[test]
    fn empty_cells_are_empty() {
        assert_eq!(record(["", "x", ""]), ",x,\r\n");
    }

    #[test]
    fn cells_with_a_comma_a_quote_or_a_line_break_are_quoted() {
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("two\nlines"), "\"two\nlines\"");
        assert_eq!(cell("cr\rhere"), "\"cr\rhere\"");
        assert_eq!(cell("plain text"), "plain text");
        assert_eq!(cell(""), "");
    }

    #[test]
    fn non_ascii_text_is_written_as_is() {
        assert_eq!(cell("Grüße, 日本"), "\"Grüße, 日本\"");
        assert_eq!(cell("Zoë"), "Zoë");
    }

    #[test]
    fn cells_that_a_spreadsheet_would_read_as_a_formula_get_a_leading_quote() {
        assert_eq!(cell("=1+1"), "'=1+1");
        assert_eq!(cell("+49 30 1234"), "'+49 30 1234");
        assert_eq!(cell("-5"), "'-5");
        assert_eq!(cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(cell("\tx"), "'\tx");
        assert_eq!(cell("\rx"), "\"'\rx\"");
        // Protected first, then quoted as needed.
        assert_eq!(
            cell("=HYPERLINK(\"x\",\"y\")"),
            "\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\""
        );
        // Only the first character counts.
        assert_eq!(cell("a=b"), "a=b");
        assert_eq!(cell("2026-12-01T12:00:00Z"), "2026-12-01T12:00:00Z");
    }
}
