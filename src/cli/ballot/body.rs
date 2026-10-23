//! Request bodies of `create` and `edit` from field flags, or from a JSON
//! file sent unchanged. Values go to the server as given; only enum values
//! are parsed, to name them in JSON.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde_json::{Map, Value, json};

use super::{Failure, enum_name, read_bytes, read_input};

/// The field flags, shared by `create` and `edit`.
#[derive(clap::Args)]
#[group(id = "fields", multiple = true)]
#[command(next_help_heading = "Ballot fields")]
pub(super) struct Fields {
    /// Title (single-line text).
    #[arg(long)]
    title: Option<String>,
    /// Description (limited Markdown).
    #[arg(long, conflicts_with = "description_file")]
    description: Option<String>,
    /// Read the description from this file (`-`: stdin).
    #[arg(long, value_name = "PATH|-")]
    description_file: Option<PathBuf>,
    /// An Option; repeat it for each Option, in display order. On `edit`,
    /// the given Options replace all Options, and each gets a new ID.
    #[arg(long = "option", value_name = "LABEL")]
    options: Vec<String>,
    /// Single or multiple choice.
    #[arg(long, value_enum)]
    choice_mode: Option<ChoiceMode>,
    /// Allow No Option: the voter may answer with no Option at all (multiple
    /// choice only). `--allow-no-option` alone means true; on `edit`,
    /// `--allow-no-option=false` turns it off.
    #[arg(
        long,
        value_name = "true|false",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_parser = clap::value_parser!(bool),
    )]
    allow_no_option: Option<bool>,
    /// Final (submitted once) or editable (changeable until the deadline).
    #[arg(long, value_enum)]
    submission_mode: Option<SubmissionMode>,
    /// Voting Deadline: RFC 3339, e.g. 2026-12-01T12:00:00+01:00; without
    /// an offset (2026-12-01T12:00) it is local time in the server's
    /// default time zone. Or a duration from now, e.g. `3d`, `1m2w`; on
    /// `edit`, `+1d` is a day after the current Voting Deadline. Units,
    /// combined without spaces (`2d12h`): min (minutes), h (hours), d
    /// (days), w (weeks), m (calendar months), y (years). Note: `m` is
    /// months here, unlike in the server's configuration.
    #[arg(long, value_name = "DATETIME|DURATION|+DURATION")]
    voting_deadline: Option<String>,
    /// Purge Date: as --voting-deadline (a duration is from now), or `+`
    /// and a duration after the Voting Deadline, e.g. `+7d`: the one given
    /// in the same command, else (on `edit`) the Ballot's current one.
    #[arg(long, value_name = "DATETIME|DURATION|+DURATION")]
    purge_date: Option<String>,
    /// An Option's label for the Fallback; repeat it for more than one.
    #[arg(long = "fallback", value_name = "LABEL")]
    fallback: Vec<String>,
    /// Group.
    #[arg(long)]
    group: Option<String>,
    /// Voter Reference (unique within its Group).
    #[arg(long)]
    voter_reference: Option<String>,
    /// Theme name.
    #[arg(long)]
    theme: Option<String>,
    /// Accent colour, #rrggbb.
    #[arg(long, value_name = "#RRGGBB")]
    accent_color: Option<String>,
    /// Logo name.
    #[arg(long)]
    logo: Option<String>,
}

/// The `--clear-*` flags of `edit`: each sends `null`, which removes the
/// value.
#[derive(clap::Args)]
#[group(id = "clears", multiple = true)]
#[command(next_help_heading = "Removing values")]
pub(super) struct Clears {
    /// Remove the description.
    #[arg(long, conflicts_with_all = ["description", "description_file"])]
    clear_description: bool,
    /// Remove the Group.
    #[arg(long, conflicts_with = "group")]
    clear_group: bool,
    /// Remove the Voter Reference.
    #[arg(long, conflicts_with = "voter_reference")]
    clear_voter_reference: bool,
    /// Remove the accent colour (the Theme's own applies).
    #[arg(long, conflicts_with = "accent_color")]
    clear_accent_color: bool,
    /// Remove the logo.
    #[arg(long, conflicts_with = "logo")]
    clear_logo: bool,
    /// Remove the Fallback.
    #[arg(long, conflicts_with = "fallback")]
    clear_fallback: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ChoiceMode {
    Single,
    Multiple,
}

#[derive(Clone, Copy, ValueEnum)]
enum SubmissionMode {
    Final,
    Editable,
}

impl Fields {
    /// The Voting Deadline and Purge Date flags, for resolving relative
    /// values before [`Self::into_body`].
    pub(super) fn dates_mut(&mut self) -> (&mut Option<String>, &mut Option<String>) {
        (&mut self.voting_deadline, &mut self.purge_date)
    }

    /// The JSON body: one key per given flag (and `null` per `--clear-*`
    /// flag); a field without a flag is left out.
    pub(super) fn into_body(self, clears: Option<Clears>) -> Result<Vec<u8>, Failure> {
        let mut body = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                body.insert(key.to_owned(), value);
            }
        };
        let description = match self.description_file {
            Some(path) => Some(read_input(&path, "--description-file")?),
            None => self.description,
        };
        put("title", self.title.map(Value::from));
        put("description", description.map(Value::from));
        put(
            "options",
            (!self.options.is_empty()).then(|| {
                (self.options.into_iter())
                    .map(|label| json!({ "label": label }))
                    .collect()
            }),
        );
        put(
            "choice_mode",
            self.choice_mode.map(enum_name).map(Value::from),
        );
        put("allow_no_option", self.allow_no_option.map(Value::from));
        put(
            "submission_mode",
            self.submission_mode.map(enum_name).map(Value::from),
        );
        put("voting_deadline", self.voting_deadline.map(Value::from));
        put("purge_date", self.purge_date.map(Value::from));
        put(
            "fallback",
            (!self.fallback.is_empty()).then(|| self.fallback.into()),
        );
        put("group", self.group.map(Value::from));
        put("voter_reference", self.voter_reference.map(Value::from));
        put("theme", self.theme.map(Value::from));
        put("accent_color", self.accent_color.map(Value::from));
        put("logo", self.logo.map(Value::from));
        if let Some(clears) = clears {
            let cleared = [
                ("description", clears.clear_description),
                ("group", clears.clear_group),
                ("voter_reference", clears.clear_voter_reference),
                ("accent_color", clears.clear_accent_color),
                ("logo", clears.clear_logo),
                ("fallback", clears.clear_fallback),
            ];
            for (key, clear) in cleared {
                put(key, clear.then_some(Value::Null));
            }
        }
        Ok(Value::Object(body).to_string().into_bytes())
    }
}

/// The body of `--body`: the file's bytes unchanged, once they are known to
/// be JSON.
pub(super) fn raw(path: &Path) -> Result<Vec<u8>, Failure> {
    let bytes = read_bytes(path, "--body")?;
    if serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_err() {
        return Err(Failure::usage("--body: the file is not JSON"));
    }
    Ok(bytes)
}
