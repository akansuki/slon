//! Turning result rows into CSV / TSV / JSON / JSON Lines text.
//!
//! One formatter serves both ways out of the app: *Export* saves the rows the grid holds, in
//! the sorted order on screen, and *Export all rows* streams every row the server returns
//! straight to disk without the grid's 100,000-row cap. Those used to be two
//! implementations — the first in TypeScript, the second here — which is the kind of pair
//! that drifts without anyone noticing, so the page now hands over rows and this decides
//! what the bytes are.
//!
//! Nothing here holds a whole file in memory: callers append a row at a time into a buffer
//! they reuse.

use serde::{Deserialize, Serialize};

use crate::query::Column;

/// One value as the server printed it. `None` is SQL NULL.
pub type Cell = Option<String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
    Jsonl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// CSV only. Anything but `,`, `;` or a tab is treated as a comma.
    pub delimiter: char,
    /// CSV only: first line holds the column names.
    pub header: bool,
    /// CSV only: UTF-8 byte order mark. Excel on Windows needs it to read accented letters.
    pub bom: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Csv,
            delimiter: ',',
            header: true,
            bom: true,
        }
    }
}

impl ExportOptions {
    /// The separator actually used. Guarded rather than trusted: the value crosses from the
    /// page, and a stray one would quietly produce a file nothing can read back.
    pub fn separator(&self) -> char {
        match self.delimiter {
            ',' | ';' | '\t' => self.delimiter,
            _ => ',',
        }
    }

    pub fn extension(&self) -> &'static str {
        match self.format {
            ExportFormat::Json => "json",
            ExportFormat::Jsonl => "jsonl",
            ExportFormat::Csv if self.separator() == '\t' => "tsv",
            ExportFormat::Csv => "csv",
        }
    }
}

/// What the formatter needs to know about a column. Deliberately smaller than [`Column`]:
/// the grid sends these back over IPC, and the rest of a `Column` is no business of a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportColumn {
    pub name: String,
    pub type_name: Option<String>,
}

impl From<&Column> for ExportColumn {
    fn from(c: &Column) -> Self {
        Self {
            name: c.name.clone(),
            type_name: c.type_name.clone(),
        }
    }
}

/* ------------------------------------------------------------------------------- CSV */

/// RFC 4180 quoting. Like PostgreSQL's own `COPY ... CSV`, NULL is an empty field and an
/// empty string is written as `""`, so the two stay apart when the file is loaded back.
pub fn csv_field(out: &mut String, value: Option<&str>, separator: char) {
    let Some(value) = value else { return };
    if value.is_empty() {
        out.push_str("\"\"");
        return;
    }
    if value.contains(separator) || value.contains(['"', '\r', '\n']) {
        out.push('"');
        for ch in value.chars() {
            if ch == '"' {
                out.push('"');
            }
            out.push(ch);
        }
        out.push('"');
    } else {
        out.push_str(value);
    }
}

fn csv_line<'a>(out: &mut String, cells: impl Iterator<Item = Option<&'a str>>, separator: char) {
    for (i, cell) in cells.enumerate() {
        if i > 0 {
            out.push(separator);
        }
        csv_field(out, cell, separator);
    }
    out.push_str("\r\n");
}

/* ------------------------------------------------------------------------------ JSON */

const NUMERIC_TYPES: [&str; 7] = ["int2", "int4", "int8", "float4", "float8", "numeric", "oid"];

/// The JSON grammar for numbers: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
///
/// Checked rather than assumed, because PostgreSQL prints `NaN` and `Infinity` as words for
/// `float8` and `numeric`, and JSON has no way to hold either. Those fall back to strings.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    // An integer part is required, and a leading zero may not be followed by more digits.
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// A JSON string literal, quotes included.
pub fn json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One value as JSON text. Numbers keep the digits the server printed (a `bigint` or
/// `numeric` would lose precision as a JavaScript number), booleans become true/false,
/// `json` and `jsonb` values are embedded as they are, everything else is a string.
pub fn json_value(out: &mut String, cell: Option<&str>, column: &ExportColumn) {
    let Some(cell) = cell else {
        out.push_str("null");
        return;
    };
    let type_name = column.type_name.as_deref().unwrap_or("");
    if NUMERIC_TYPES.contains(&type_name) && is_json_number(cell) {
        out.push_str(cell);
        return;
    }
    match (type_name, cell) {
        ("bool", "t") => return out.push_str("true"),
        ("bool", "f") => return out.push_str("false"),
        // Whatever PostgreSQL stored in a json/jsonb column was valid JSON when it went in,
        // so it is embedded rather than quoted into a string of itself.
        ("json" | "jsonb", _) => return out.push_str(cell),
        _ => {}
    }
    json_string(out, cell);
}

/// Column names as object keys, pre-quoted. A repeated name gets `_2`, `_3`, ... so that a
/// `SELECT a.id, b.id` keeps both values instead of one overwriting the other.
pub fn json_keys(columns: &[ExportColumn]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(columns.len());
    let mut keys = Vec::with_capacity(columns.len());
    for c in columns {
        let mut key = c.name.clone();
        let mut n = 2;
        while taken.contains(&key) {
            key = format!("{}_{n}", c.name);
            n += 1;
        }
        taken.push(key.clone());
        let mut quoted = String::new();
        json_string(&mut quoted, &key);
        keys.push(quoted);
    }
    keys
}

fn json_object(out: &mut String, row: &[Cell], columns: &[ExportColumn], keys: &[String]) {
    out.push('{');
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&keys[i]);
        out.push_str(": ");
        json_value(out, row.get(i).and_then(Option::as_deref), column);
    }
    out.push('}');
}

/* ------------------------------------------------------------------- pieces of a file */

/// Text that opens the file.
pub fn export_header(columns: &[ExportColumn], o: &ExportOptions) -> String {
    let mut out = String::new();
    match o.format {
        ExportFormat::Csv => {
            if o.bom {
                out.push('\u{feff}');
            }
            if o.header {
                csv_line(
                    &mut out,
                    columns.iter().map(|c| Some(c.name.as_str())),
                    o.separator(),
                );
            }
        }
        ExportFormat::Json => out.push('['),
        ExportFormat::Jsonl => {}
    }
    out
}

/// Append one row. `index` is its position in the whole file, which is what decides whether
/// a JSON array element needs a comma in front of it.
pub fn export_row(
    out: &mut String,
    cells: &[Cell],
    columns: &[ExportColumn],
    keys: &[String],
    o: &ExportOptions,
    index: u64,
) {
    match o.format {
        ExportFormat::Csv => csv_line(
            out,
            (0..columns.len()).map(|i| cells.get(i).and_then(Option::as_deref)),
            o.separator(),
        ),
        ExportFormat::Json => {
            out.push_str(if index == 0 { "\n  " } else { ",\n  " });
            json_object(out, cells, columns, keys);
        }
        ExportFormat::Jsonl => {
            json_object(out, cells, columns, keys);
            out.push('\n');
        }
    }
}

/// Text that closes the file.
pub fn export_footer(row_count: u64, o: &ExportOptions) -> String {
    match o.format {
        ExportFormat::Json if row_count > 0 => "\n]\n".into(),
        ExportFormat::Json => "]\n".into(),
        _ => String::new(),
    }
}

/// The whole thing as one string. For small results and tests; the streaming paths append a
/// row at a time into a buffer they reuse.
pub fn export_all(columns: &[ExportColumn], rows: &[Vec<Cell>], o: &ExportOptions) -> String {
    let keys = json_keys(columns);
    let mut out = export_header(columns, o);
    for (i, cells) in rows.iter().enumerate() {
        export_row(&mut out, cells, columns, &keys, o, i as u64);
    }
    out.push_str(&export_footer(rows.len() as u64, o));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> ExportColumn {
        ExportColumn {
            name: name.into(),
            type_name: None,
        }
    }

    fn typed(name: &str, type_name: &str) -> ExportColumn {
        ExportColumn {
            name: name.into(),
            type_name: Some(type_name.into()),
        }
    }

    fn s(v: &str) -> Cell {
        Some(v.into())
    }

    fn opts(format: ExportFormat) -> ExportOptions {
        ExportOptions {
            format,
            bom: false,
            ..Default::default()
        }
    }

    fn field(value: Option<&str>, separator: char) -> String {
        let mut out = String::new();
        csv_field(&mut out, value, separator);
        out
    }

    #[test]
    fn csv_keeps_null_and_the_empty_string_apart() {
        assert_eq!(field(None, ','), "");
        assert_eq!(field(Some(""), ','), "\"\"");
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        assert_eq!(field(Some("plain"), ','), "plain");
        assert_eq!(field(Some("a,b"), ','), "\"a,b\"");
        assert_eq!(field(Some("a,b"), ';'), "a,b");
        assert_eq!(field(Some("a;b"), ';'), "\"a;b\"");
        assert_eq!(field(Some("say \"hi\""), ','), "\"say \"\"hi\"\"\"");
        assert_eq!(field(Some("two\nlines"), ','), "\"two\nlines\"");
        assert_eq!(field(Some("ends\r"), ','), "\"ends\r\"");
        // A tab needs no quoting in a comma file, and does in a tab file.
        assert_eq!(field(Some("a\tb"), ','), "a\tb");
        assert_eq!(field(Some("a\tb"), '\t'), "\"a\tb\"");
    }

    #[test]
    fn an_unknown_delimiter_falls_back_to_a_comma() {
        let o = ExportOptions {
            delimiter: '"',
            ..Default::default()
        };
        assert_eq!(o.separator(), ',');
    }

    #[test]
    fn json_numbers_keep_the_digits_the_server_sent() {
        let mut out = String::new();
        // Beyond 2^53: as a JavaScript number this would come back as ...448.
        json_value(&mut out, Some("9007199254740993"), &typed("n", "int8"));
        assert_eq!(out, "9007199254740993");

        out.clear();
        json_value(&mut out, Some("1.50"), &typed("n", "numeric"));
        assert_eq!(out, "1.50");
    }

    #[test]
    fn json_falls_back_to_strings_for_numbers_json_cannot_hold() {
        for value in ["NaN", "Infinity", "-Infinity"] {
            let mut out = String::new();
            json_value(&mut out, Some(value), &typed("n", "float8"));
            assert_eq!(out, format!("\"{value}\""), "{value}");
        }
    }

    #[test]
    fn json_number_grammar() {
        for ok in ["0", "-0", "12", "-3.5", "1e9", "1E+9", "2.5e-3", "0.5"] {
            assert!(is_json_number(ok), "{ok} should be a number");
        }
        for bad in [
            "", "-", "01", "1.", ".5", "1e", "1e+", "+1", "1 ", " 1", "0x10", "1,5", "NaN",
        ] {
            assert!(!is_json_number(bad), "{bad} should not be a number");
        }
    }

    #[test]
    fn json_types_become_json_and_bools_become_bools() {
        let mut out = String::new();
        json_value(&mut out, Some(r#"{"a": 1}"#), &typed("j", "jsonb"));
        assert_eq!(out, r#"{"a": 1}"#);

        out.clear();
        json_value(&mut out, Some("t"), &typed("b", "bool"));
        assert_eq!(out, "true");
        out.clear();
        json_value(&mut out, Some("f"), &typed("b", "bool"));
        assert_eq!(out, "false");
    }

    #[test]
    fn json_escapes_control_characters_and_quotes() {
        let mut out = String::new();
        json_string(&mut out, "a\"b\\c\nd\te\u{1}f");
        assert_eq!(out, r#""a\"b\\c\nd\te\u0001f""#);

        // Accented letters and emoji are left alone; the file is UTF-8.
        out.clear();
        json_string(&mut out, "Čađavi 🐘");
        assert_eq!(out, "\"Čađavi 🐘\"");
    }

    #[test]
    fn repeated_column_names_each_keep_their_value() {
        let keys = json_keys(&[col("id"), col("id"), col("id_2"), col("name")]);
        assert_eq!(keys, [r#""id""#, r#""id_2""#, r#""id_2_2""#, r#""name""#]);
    }

    #[test]
    fn csv_file_has_a_header_and_crlf_line_ends() {
        let o = ExportOptions {
            bom: false,
            ..Default::default()
        };
        let text = export_all(
            &[col("a,b"), col("c")],
            &[vec![s("1"), None], vec![s(""), s("x")]],
            &o,
        );
        assert_eq!(text, "\"a,b\",c\r\n1,\r\n\"\",x\r\n");
    }

    #[test]
    fn the_byte_order_mark_is_only_on_csv() {
        let o = ExportOptions::default();
        assert!(export_all(&[col("a")], &[], &o).starts_with('\u{feff}'));
        assert!(!export_all(&[col("a")], &[], &opts(ExportFormat::Json)).starts_with('\u{feff}'));
    }

    #[test]
    fn json_is_an_array_of_objects() {
        let text = export_all(
            &[typed("id", "int4"), col("name")],
            &[vec![s("1"), s("a")], vec![s("2"), None]],
            &opts(ExportFormat::Json),
        );
        assert_eq!(
            text,
            "[\n  {\"id\": 1, \"name\": \"a\"},\n  {\"id\": 2, \"name\": null}\n]\n"
        );
    }

    #[test]
    fn an_empty_json_result_is_still_a_valid_document() {
        assert_eq!(
            export_all(&[col("a")], &[], &opts(ExportFormat::Json)),
            "[]\n"
        );
        assert_eq!(export_all(&[col("a")], &[], &opts(ExportFormat::Jsonl)), "");
    }

    #[test]
    fn json_lines_is_one_object_per_line_with_no_wrapper() {
        let text = export_all(
            &[typed("id", "int4")],
            &[vec![s("1")], vec![s("2")]],
            &opts(ExportFormat::Jsonl),
        );
        assert_eq!(text, "{\"id\": 1}\n{\"id\": 2}\n");
    }

    /// Writing the file in pieces must produce the same bytes as writing it in one go,
    /// because that is the only difference between the two export paths.
    #[test]
    fn piecewise_output_matches_the_whole_thing() {
        let columns = [typed("id", "int4"), col("name")];
        let rows: Vec<Vec<Cell>> = (0..7)
            .map(|i| vec![s(&i.to_string()), s(&format!("row {i}"))])
            .collect();
        for format in [ExportFormat::Csv, ExportFormat::Json, ExportFormat::Jsonl] {
            let o = opts(format);
            let keys = json_keys(&columns);
            let mut piecewise = export_header(&columns, &o);
            for (i, cells) in rows.iter().enumerate() {
                export_row(&mut piecewise, cells, &columns, &keys, &o, i as u64);
            }
            piecewise.push_str(&export_footer(rows.len() as u64, &o));
            assert_eq!(piecewise, export_all(&columns, &rows, &o), "{format:?}");
        }
    }

    #[test]
    fn a_short_row_is_padded_with_nulls_rather_than_shifting_columns() {
        let columns = [col("a"), col("b")];
        let o = opts(ExportFormat::Jsonl);
        assert_eq!(
            export_all(&columns, &[vec![s("1")]], &o),
            "{\"a\": \"1\", \"b\": null}\n"
        );
        assert_eq!(
            export_all(&columns, &[vec![s("1")]], &opts(ExportFormat::Csv)),
            "a,b\r\n1,\r\n"
        );
    }
}
