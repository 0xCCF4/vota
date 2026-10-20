//! The trial render: every Theme's templates rendered once per voting page
//! state at startup, with sample Ballots, so a template that parses but
//! fails when rendered (an `extends` of a missing template, an unknown
//! filter, …) stops startup instead of giving voters an error page later.
//! So does a voting page that takes a Response, or a confirmation step,
//! without a `<form method="post">` the app can put the hidden `revision`
//! field in (`revision_field`): a voter without JavaScript couldn't answer.
//! And so does a POST form with a `select`, `textarea` or `button` named
//! `revision`: with the inserted field it would post `revision` twice.
//!
//! The contexts come from the same builders the real pages use
//! ([`ballot_page::voting_page_context`],
//! [`ballot_page::confirmation_context`]), so the trial can't drift from
//! what the pages pass. The sample values are obviously fake: there are no
//! Ballot Tokens or Voter References at startup to leak.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use minijinja::Environment;

use super::ballot_page;
use super::revision_field;
use super::themes::Look;
use crate::domain::ballot::{
    Ballot, BallotOption, BallotToken, ChoiceMode, OptionId, Response, SubmissionMode,
};
use crate::domain::theme::AccentColor;

/// A failed trial render.
#[derive(Debug)]
pub(super) struct Failure {
    /// The template that failed: where minijinja places the error, or else
    /// the one rendered.
    pub(super) template: String,
    /// Which sample page state it failed on.
    pub(super) sample: String,
    pub(super) problem: Problem,
}

/// Why a trial render failed.
#[derive(Debug)]
pub(super) enum Problem {
    /// The template doesn't render.
    Render(minijinja::Error),
    /// A page that takes a Response rendered without a `<form
    /// method="post">` for the app to put the hidden `revision` field in.
    NoPostForm,
    /// A `<form method="post">` has this other control (`select`,
    /// `textarea` or `button`) named `revision`, so with the app's hidden
    /// field it would post `revision` twice.
    RevisionControl(&'static str),
}

/// Renders `templates` (the Theme `theme`'s) for every voting page state:
/// `ballot.html` open (single and multiple choice, Final and Editable,
/// unanswered, answered), closed (with a Response, with the Fallback
/// applied, without an answer) and `confirm.html`, each with and without
/// description, logo, accent colour, notice and base path other than `/`
/// (and, while open and unanswered, with and without a Fallback), so a
/// Theme that only breaks under a prefix fails even on a root deployment.
/// `base.html` is rendered through them. The pages that take a Response
/// (open and unanswered, or Editable) and `confirm.html` must have a
/// `<form method="post">`.
pub(super) fn check(theme: &str, templates: &Environment<'_>) -> Result<(), Failure> {
    for decorated in [false, true] {
        let look = Look {
            theme,
            logo: decorated.then_some("sample-logo"),
            base: if decorated { SAMPLE_BASE_PATH } else { "/" },
        };
        for (state, ballot, now) in voting_page_samples(decorated) {
            let message = decorated.then_some("Sample notice for the voter.");
            let context =
                ballot_page::voting_page_context(&ballot, now, Tz::UTC, message, &look, NONCE);
            // The page has the form exactly while it takes a Response.
            let needs_form = ballot.accepts_response(now).is_ok();
            render(templates, "ballot.html", context, needs_form, || {
                sample_name(state, decorated)
            })?;
        }
        for choice_mode in ChoiceMode::ALL {
            let ballot = sample_ballot(choice_mode, SubmissionMode::Final, decorated);
            // As chosen by the voter: the first Option, and on a multiple
            // choice Ballot the second, too.
            let selected: Vec<OptionId> = ballot
                .options
                .iter()
                .take(match choice_mode {
                    ChoiceMode::Single => 1,
                    ChoiceMode::Multiple => 2,
                })
                .map(|o| o.id.clone())
                .collect();
            let context = ballot_page::confirmation_context(&ballot, &selected, &look, NONCE);
            render(templates, "confirm.html", context, true, || {
                sample_name(
                    &format!("confirmation, {}", choice_mode.as_str()),
                    decorated,
                )
            })?;
        }
    }
    Ok(())
}

const NONCE: &str = "sample-nonce";

/// A base path as under a prefix, of more than one segment.
const SAMPLE_BASE_PATH: &str = "/sample/base-path/";

fn sample_name(state: &str, decorated: bool) -> String {
    let extras = if decorated {
        "with description, logo, accent colour, notice and base path /sample/base-path/"
    } else {
        "without description, logo, accent colour or notice, at base path /"
    };
    format!("{state}, {extras}")
}

/// Renders `template`; with `needs_form`, the result must have a `<form
/// method="post">` the app can put the hidden `revision` field in. No POST
/// form may have a `select`, `textarea` or `button` named `revision`.
fn render(
    templates: &Environment<'_>,
    template: &str,
    context: minijinja::Value,
    needs_form: bool,
    sample: impl FnOnce() -> String,
) -> Result<(), Failure> {
    let problem = match templates
        .get_template(template)
        .and_then(|t| t.render(context))
    {
        Err(error) => Problem::Render(error),
        Ok(html) if needs_form && !revision_field::has_post_form(&html) => Problem::NoPostForm,
        Ok(html) => match revision_field::revision_control(&html) {
            Some(control) => Problem::RevisionControl(control),
            None => return Ok(()),
        },
    };
    let template = match &problem {
        Problem::Render(error) => error.name().unwrap_or(template),
        Problem::NoPostForm | Problem::RevisionControl(_) => template,
    };
    Err(Failure {
        template: template.to_owned(),
        sample: sample(),
        problem,
    })
}

/// Every state of the voting page: the sample Ballot and the "now" to
/// render it at.
fn voting_page_samples(decorated: bool) -> Vec<(&'static str, Ballot, DateTime<Utc>)> {
    let open = sample_deadline() - Duration::days(1);
    let closed = sample_deadline() + Duration::days(1);
    let unanswered =
        |choice_mode, submission_mode| sample_ballot(choice_mode, submission_mode, decorated);
    let answered = |submission_mode| {
        let mut ballot = sample_ballot(ChoiceMode::Single, submission_mode, decorated);
        ballot.response = Some(Response {
            option_ids: vec![ballot.options[0].id.clone()],
            answered_at: open,
            changed_at: open,
        });
        ballot
    };
    vec![
        (
            "open, single choice, Final",
            unanswered(ChoiceMode::Single, SubmissionMode::Final),
            open,
        ),
        (
            "open, multiple choice, Final",
            unanswered(ChoiceMode::Multiple, SubmissionMode::Final),
            open,
        ),
        (
            "open, single choice, Editable",
            unanswered(ChoiceMode::Single, SubmissionMode::Editable),
            open,
        ),
        (
            "open, multiple choice, Editable",
            unanswered(ChoiceMode::Multiple, SubmissionMode::Editable),
            open,
        ),
        (
            "Editable, answered",
            answered(SubmissionMode::Editable),
            open,
        ),
        ("Final, answered", answered(SubmissionMode::Final), open),
        (
            "closed with a Response",
            answered(SubmissionMode::Final),
            closed,
        ),
        (
            "closed with the Fallback applied",
            with_fallback(unanswered(ChoiceMode::Multiple, SubmissionMode::Final)),
            closed,
        ),
        (
            "closed without an answer",
            Ballot {
                fallback: None,
                ..unanswered(ChoiceMode::Single, SubmissionMode::Final)
            },
            closed,
        ),
    ]
}

fn sample_deadline() -> DateTime<Utc> {
    DateTime::from_timestamp(946_782_000, 0).expect("a valid timestamp") // 2000-01-02
}

/// A sample Ballot with three Options; decorated, with a description, an
/// accent colour and a Fallback (the logo is the [`Look`]'s). Unanswered.
fn sample_ballot(
    choice_mode: ChoiceMode,
    submission_mode: SubmissionMode,
    decorated: bool,
) -> Ballot {
    let options: Vec<BallotOption> = ["a", "b", "c"]
        .into_iter()
        .map(|id| BallotOption {
            // Samples, never stored: the IDs only have to be distinct.
            id: OptionId::from_store(format!("sample-option-{id}")),
            label: format!("Sample Option {}", id.to_uppercase()),
        })
        .collect();
    let ballot = Ballot {
        id: 0,
        token: BallotToken::from_store("not-a-real-ballot-token".to_owned()),
        title: "Sample Ballot".to_owned(),
        description: decorated.then(|| "A *sample* description with a [link](/).".to_owned()),
        options,
        choice_mode,
        submission_mode,
        voting_deadline: sample_deadline(),
        purge_date: sample_deadline() + Duration::days(30),
        created_at: sample_deadline() - Duration::days(7),
        response: None,
        fallback: None,
        group: None,
        voter_reference: None,
        theme: String::new(),
        accent_color: decorated.then(|| AccentColor::from_store("#3366cc".to_owned())),
        logo: None,
        revision: 0,
    };
    if decorated {
        with_fallback(ballot)
    } else {
        ballot
    }
}

/// `ballot` with a Fallback obeying its Choice Mode.
fn with_fallback(ballot: Ballot) -> Ballot {
    let ids = match ballot.choice_mode {
        ChoiceMode::Single => vec![ballot.options[1].id.clone()],
        ChoiceMode::Multiple => vec![ballot.options[1].id.clone(), ballot.options[2].id.clone()],
    };
    Ballot {
        fallback: Some(ids),
        ..ballot
    }
}
