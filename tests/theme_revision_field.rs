//! The hidden `revision` field of the voting and confirmation forms is
//! inserted by the app after rendering, so a folder Theme overriding
//! `ballot.html` or `confirm.html` can't forget it: every `<form
//! method="post">` gets exactly one, with the Ballot's current revision.
//! A Theme whose open voting page or confirmation step has no such form,
//! or whose POST form has a `select`, `textarea` or `button` named
//! `revision`, fails at startup.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_string, write_file};
use serde_json::json;

/// The app with these files in its theme folder.
async fn app_with_themes(files: &[(&str, &str)]) -> TestApp {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect();
    TestApp::with_config(move |config| {
        let folder = config.theme_folder.clone().unwrap();
        for (path, contents) in files {
            write_file(&folder, &path, contents);
        }
    })
    .await
}

/// Creates a single-choice Ballot with this Theme and Submission Mode and
/// edits its title once, so its revision is 1; returns its Ballot Token and
/// Option IDs.
async fn ballot_at_revision_1(
    app: &TestApp,
    theme: &str,
    submission_mode: &str,
) -> (String, Vec<String>) {
    let mut request = ballot_request();
    request["theme"] = json!(theme);
    request["submission_mode"] = json!(submission_mode);
    let created = app.create_ballot(&request).await;
    let admin = app.admin_token().await;
    let edited = app
        .admin_patch(
            &format!("/admin/ballots/{}", created["id"]),
            &admin,
            &json!({ "title": "Lunch on Saturday?" }),
        )
        .await;
    assert_eq!(edited.status(), StatusCode::OK);
    let options = created["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    (created["token"].as_str().unwrap().to_owned(), options)
}

/// The values of the page's hidden `revision` fields as the app inserts
/// them, in order.
fn injected_revisions(html: &str) -> Vec<&str> {
    let marker = r#"<input type="hidden" name="revision" value=""#;
    html.match_indices(marker)
        .map(|(at, _)| {
            let rest = &html[at + marker.len()..];
            &rest[..rest.find('"').unwrap()]
        })
        .collect()
}

#[tokio::test]
async fn a_voting_page_override_without_the_field_gets_it_and_a_no_js_submit_is_recorded() {
    let ballot = r#"{% extends "base.html" %}{% block content %}<h1>{{ title }}</h1>
<form method="post" action="">
{% for option in options %}<input type="radio" name="option" value="{{ option.id }}" id="o-{{ option.id }}"><label for="o-{{ option.id }}">{{ option.label }}</label>{% endfor %}
<button type="submit">Save</button>
</form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("bare/style.css", ""),
        ("bare/templates/ballot.html", ballot),
    ])
    .await;
    let (token, options) = ballot_at_revision_1(&app, "bare", "editable").await;
    let page = format!("/b/{token}");

    let html = body_string(app.get(&page).await).await;

    assert_eq!(injected_revisions(&html), ["1"], "{html}");
    // As a browser without JavaScript submits the form: with the field it
    // found in it.
    let response = app
        .post_form(&page, &format!("option={}&revision=1", options[0]))
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(app.voter_view(&token).await["status"], "answered");
}

/// How many fields named `revision` the page has, however they are quoted.
fn revision_fields(html: &str) -> usize {
    [r#"name="revision""#, "name='revision'", "name=revision"]
        .iter()
        .map(|name| html.matches(name).count())
        .sum()
}

#[tokio::test]
async fn an_override_that_has_the_field_keeps_it_without_a_duplicate() {
    let ballot = r#"{% extends "base.html" %}{% block content %}
<form method="post" action="">
<input type="hidden" name="revision" value="{{ revision }}">
{% for option in options %}<input type="radio" name="option" value="{{ option.id }}" id="o-{{ option.id }}"><label for="o-{{ option.id }}">{{ option.label }}</label>{% endfor %}
<button type="submit">Continue</button>
</form>{% endblock %}"#;
    let confirm = r#"{% extends "base.html" %}{% block content %}
<form method="post" action="">
{% for option in selected %}<input type="hidden" name="option" value="{{ option.id }}">{% endfor %}
<input type="hidden" name="revision" value="{{ revision }}">
<button type="submit" name="confirm" value="yes">Confirm</button>
</form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("older/style.css", ""),
        ("older/templates/ballot.html", ballot),
        ("older/templates/confirm.html", confirm),
    ])
    .await;
    let (token, options) = ballot_at_revision_1(&app, "older", "final").await;
    let page = format!("/b/{token}");

    let html = body_string(app.get(&page).await).await;
    assert_eq!(revision_fields(&html), 1, "{html}");
    assert_eq!(injected_revisions(&html), ["1"], "{html}");

    let confirmation = app
        .post_form(&page, &format!("option={}&revision=1", options[0]))
        .await;
    assert_eq!(confirmation.status(), StatusCode::OK);
    let confirmation = body_string(confirmation).await;
    assert_eq!(revision_fields(&confirmation), 1, "{confirmation}");
    assert_eq!(injected_revisions(&confirmation), ["1"], "{confirmation}");
}

#[tokio::test]
async fn a_confirmation_override_without_the_field_gets_it_and_a_no_js_confirmation_is_recorded() {
    let confirm = r#"{% extends "base.html" %}{% block content %}<h2>Sure?</h2>
<form method="post" action="">
{% for option in selected %}<input type="hidden" name="option" value="{{ option.id }}">{% endfor %}
<button type="submit" name="confirm" value="yes">Confirm</button>
</form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("bare/style.css", ""),
        ("bare/templates/confirm.html", confirm),
    ])
    .await;
    let (token, options) = ballot_at_revision_1(&app, "bare", "final").await;
    let page = format!("/b/{token}");

    let confirmation = app
        .post_form(&page, &format!("option={}&revision=1", options[0]))
        .await;

    assert_eq!(confirmation.status(), StatusCode::OK);
    let confirmation = body_string(confirmation).await;
    assert!(confirmation.contains("<h2>Sure?</h2>"), "{confirmation}");
    assert_eq!(injected_revisions(&confirmation), ["1"], "{confirmation}");
    // As a browser without JavaScript posts the confirmation form.
    let response = app
        .post_form(
            &page,
            &format!("option={}&confirm=yes&revision=1", options[0]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(app.voter_view(&token).await["status"], "answered");
}

#[tokio::test]
async fn unusual_but_valid_markup_gets_one_field_per_post_form_and_nothing_else_is_touched() {
    // Upper case, single, no or spaced quotes, attributes in any order, a
    // `>` inside an attribute value, several forms; and things that look
    // like forms but aren't: in a comment, in raw text, in an attribute
    // value, a GET form, and a form start tag nested in a form (which a
    // browser ignores).
    let ballot = r#"{% extends "base.html" %}{% block content %}
<!-- <form method="post"> commented out -->
<p title='<form method="post">'>Pick one</p>
<textarea name="note"><form method="post"></textarea>
<form method="get" action=""><input name="q"></form>
<FORM ACTION='' data-hint="a > b" METHOD=POST
  ><select name="option">{% for option in options %}<option value="{{ option.id }}">{{ option.label }}</option>{% endfor %}</select>
<form method="post"><button>Save</button></FORM>
<form
    method = 'Post'
    class="second"><input type=hidden name=revision value={{ revision }}><button>Save too</button></form>
<form method="post"/><button>Also</button></form>
{% endblock %}"#;
    let app =
        app_with_themes(&[("odd/style.css", ""), ("odd/templates/ballot.html", ballot)]).await;
    let (token, _) = ballot_at_revision_1(&app, "odd", "editable").await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    let field = r#"<input type="hidden" name="revision" value="1">"#;
    assert!(
        html.contains(&format!(
            "<FORM ACTION='' data-hint=\"a > b\" METHOD=POST\n  >{field}<select"
        )),
        "{html}"
    );
    assert!(
        html.contains(&format!("<form method=\"post\"/>{field}<button>Also")),
        "{html}"
    );
    // The second form had its own; nested, commented, raw-text, attribute
    // and GET "forms" got none.
    assert_eq!(injected_revisions(&html), ["1", "1"], "{html}");
    assert_eq!(revision_fields(&html), 3, "{html}");
}

/// The startup error for a theme folder with these files.
async fn startup_error(files: &[(&str, &str)]) -> String {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect();
    let started = TestApp::try_with_config(move |config| {
        let folder = config.theme_folder.clone().unwrap();
        for (path, contents) in files {
            write_file(&folder, &path, contents);
        }
    })
    .await;
    match started {
        Ok(_) => panic!("startup should fail"),
        Err(error) => format!("{error:#}"),
    }
}

#[tokio::test]
async fn a_voting_page_without_a_post_form_fails_startup_naming_theme_and_template() {
    let error = startup_error(&[
        ("formless/style.css", ""),
        (
            "formless/templates/ballot.html",
            r#"{% extends "base.html" %}{% block content %}
{% for option in options %}<p>{{ option.label }}</p>{% endfor %}
<form method="get" action=""><button>Search</button></form>
<!-- <form method="post"></form> -->{% endblock %}"#,
        ),
    ])
    .await;

    assert!(error.contains(r#"Theme "formless""#), "{error}");
    assert!(error.contains("formless/templates/ballot.html"), "{error}");
    assert!(error.contains(r#"<form method="post">"#), "{error}");
    // The first sample page that needs the form.
    assert!(error.contains("open, single choice, Final"), "{error}");
}

#[tokio::test]
async fn a_confirmation_step_without_a_post_form_fails_startup_naming_theme_and_template() {
    let error = startup_error(&[
        ("formless/style.css", ""),
        (
            "formless/templates/confirm.html",
            r#"{% extends "base.html" %}{% block content %}<p>Please call us to confirm.</p>{% endblock %}"#,
        ),
    ])
    .await;

    assert!(error.contains(r#"Theme "formless""#), "{error}");
    assert!(error.contains("formless/templates/confirm.html"), "{error}");
    assert!(error.contains("confirmation, single"), "{error}");
}

#[tokio::test]
async fn a_base_template_that_drops_the_form_fails_startup_naming_it() {
    let error = startup_error(&[
        ("formless/style.css", ""),
        (
            "formless/templates/base.html",
            "<html><body>{% block title %}{% endblock %}</body></html>",
        ),
    ])
    .await;

    assert!(error.contains(r#"Theme "formless""#), "{error}");
    assert!(error.contains("built-in ballot.html"), "{error}");
    assert!(error.contains("formless/templates/base.html"), "{error}");
}

#[tokio::test]
async fn a_post_form_with_a_select_textarea_or_button_named_revision_fails_startup_naming_it() {
    for (template, control, element) in [
        (
            "ballot.html",
            r#"<button type="submit" name="revision" value="1">Save</button>"#,
            "button",
        ),
        (
            "ballot.html",
            "<SELECT NAME=revision><option>1</option></SELECT>",
            "select",
        ),
        (
            "confirm.html",
            "<textarea name='revision'>1</textarea>",
            "textarea",
        ),
    ] {
        let page = format!(
            r#"{{% extends "base.html" %}}{{% block content %}}
<form method="post" action="">
{{% for option in options %}}<input type="radio" name="option" value="{{{{ option.id }}}}">{{% endfor %}}
{control}
</form>{{% endblock %}}"#
        );
        let error = startup_error(&[
            ("clash/style.css", ""),
            (&format!("clash/templates/{template}"), &page),
        ])
        .await;

        assert!(error.contains(r#"Theme "clash""#), "{error}");
        assert!(
            error.contains(&format!("clash/templates/{template}")),
            "{error}"
        );
        assert!(
            error.contains(&format!(r#"a <{element}> named "revision""#)),
            "{error}"
        );
    }
}

#[tokio::test]
async fn controls_named_revision_outside_a_post_form_are_fine() {
    let ballot = r#"{% extends "base.html" %}{% block content %}
<form method="get" action=""><button name="revision">Reload</button></form>
<select name="revision"><option>1</option></select>
<!-- <form method="post"><button name="revision"></button></form> -->
<form method="post" action="">
{% for option in options %}<input type="radio" name="option" value="{{ option.id }}">{% endfor %}
<button type="submit">Save</button>
</form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("outside/style.css", ""),
        ("outside/templates/ballot.html", ballot),
    ])
    .await;
    let (token, _) = ballot_at_revision_1(&app, "outside", "editable").await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert_eq!(injected_revisions(&html), ["1"], "{html}");
}

#[tokio::test]
async fn the_built_in_voting_page_and_confirmation_step_have_exactly_one_field() {
    let app = TestApp::new().await;
    let (token, options) = ballot_at_revision_1(&app, "classic", "final").await;
    let page = format!("/b/{token}");

    let html = body_string(app.get(&page).await).await;
    let confirmation = body_string(
        app.post_form(&page, &format!("option={}&revision=1", options[0]))
            .await,
    )
    .await;

    assert_eq!(revision_fields(&html), 1, "{html}");
    assert_eq!(injected_revisions(&html), ["1"], "{html}");
    assert_eq!(revision_fields(&confirmation), 1, "{confirmation}");
    assert_eq!(injected_revisions(&confirmation), ["1"], "{confirmation}");
}
