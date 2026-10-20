//! Markdown descriptions: optional on creation and edit, stored as the
//! sanitised Markdown source, returned as source by the admin and voter APIs,
//! and rendered (raw HTML dropped, output sanitised) on the voting page.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json, body_string};
use serde_json::{Value, json};

fn with_description(description: impl Into<Value>) -> Value {
    let mut request = ballot_request();
    request["description"] = description.into();
    request
}

/// The voting page of `token`.
async fn page(app: &TestApp, token: &str) -> String {
    body_string(app.get(&format!("/b/{token}")).await).await
}

/// The rendered description block of a voting page (`div` is not among the
/// allowed tags, so the first `</div>` closes it); empty if there is none.
fn description_html(page: &str) -> &str {
    let Some(start) = page.find(r#"<div class="description">"#) else {
        return "";
    };
    let rest = &page[start + r#"<div class="description">"#.len()..];
    &rest[..rest.find("</div>").unwrap()]
}

/// Creates a Ballot with this description and returns its rendered
/// description block on the voting page.
async fn rendered(app: &TestApp, markdown: &str) -> String {
    let created = app.create_ballot(&with_description(markdown)).await;
    let page = page(app, created["token"].as_str().unwrap()).await;
    description_html(&page).trim().to_owned()
}

#[tokio::test]
async fn admin_sets_a_description_at_creation() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&with_description("Bring **snacks**."))
        .await;

    assert_eq!(created["description"], "Bring **snacks**.", "{created}");
}

#[tokio::test]
async fn the_voter_api_returns_the_markdown_source_not_html() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&with_description("Bring **snacks**."))
        .await;

    let voter = app.voter_view(created["token"].as_str().unwrap()).await;

    assert_eq!(voter["description"], "Bring **snacks**.", "{voter}");
}

#[tokio::test]
async fn the_description_is_optional() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created.get("description"), Some(&json!(null)), "{created}");
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter.get("description"), Some(&json!(null)), "{voter}");
}

#[tokio::test]
async fn the_voting_page_renders_bold_and_italic() {
    let app = TestApp::new().await;

    let html = rendered(&app, "Bring **snacks** and *drinks*.").await;

    assert_eq!(
        html,
        "<p>Bring <strong>snacks</strong> and <em>drinks</em>.</p>"
    );
}

/// Asserts that `html` holds only the allowed markup: tags from the allowlist,
/// and attributes only on links (`href` with an http, https or mailto URL, and
/// the fixed `rel`).
fn assert_only_allowed_markup(html: &str, source: &str) {
    const ALLOWED: &[&str] = &[
        "p",
        "br",
        "em",
        "strong",
        "a",
        "ul",
        "ol",
        "li",
        "code",
        "blockquote",
    ];
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let end = rest[start..]
            .find('>')
            .unwrap_or_else(|| panic!("unclosed tag in {html:?} from {source:?}"))
            + start;
        let tag = &rest[start + 1..end];
        let tag = tag.strip_prefix('/').unwrap_or(tag);
        let (name, attributes) = tag.split_once(' ').unwrap_or((tag, ""));
        assert!(
            ALLOWED.contains(&name),
            "tag <{name}> in {html:?} from {source:?}"
        );
        if !attributes.is_empty() {
            assert_eq!(
                name, "a",
                "attributes on <{name}> in {html:?} from {source:?}"
            );
            let href = attributes
                .strip_prefix("href=\"")
                .and_then(|a| a.strip_suffix("\" rel=\"noopener noreferrer nofollow\""))
                .unwrap_or_else(|| panic!("link attributes {attributes:?} from {source:?}"));
            assert!(
                ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|scheme| href.starts_with(scheme))
                    && !href.contains('"'),
                "link target {href:?} from {source:?}"
            );
        }
        rest = &rest[end + 1..];
    }
}

#[tokio::test]
async fn raw_html_is_dropped() {
    let app = TestApp::new().await;
    let attacks = [
        "<script>alert(1)</script>",
        "Hi <script>alert(1)</script> there",
        "<SCRIPT SRC=https://evil.example/x.js></SCRIPT>",
        "<img src=x onerror=alert(1)>",
        "Look: <img src=x onerror=alert(1)>",
        r#"<a href="javascript:alert(1)">click</a>"#,
        r#"Inline <a href="javascript:alert(1)">click</a>"#,
        r#"<iframe src="https://evil.example"></iframe>"#,
        "<style>body { display: none }</style>",
        "Text <style>*{color:red}</style>",
        "<div onclick=alert(1)>**bold**</div>",
        "<!-- comment --> after",
        "<svg/onload=alert(1)>",
        "</div><script>alert(1)</script>",
    ];
    for attack in attacks {
        let html = rendered(&app, attack).await;
        assert_only_allowed_markup(&html, attack);
    }
}

#[tokio::test]
async fn links_other_than_http_https_and_mailto_are_neutralised() {
    let app = TestApp::new().await;
    let attacks = [
        "[x](javascript:alert(1))",
        "[x](JaVaScRiPt:alert(1))",
        "[x](javascript&#58;alert(1))",
        "[x](&#106;avascript:alert(1))",
        "[x](javascript%3Aalert(1))",
        "[x]( javascript:alert(1) )",
        "[x](<javascript:alert(1)>)",
        "[x](vbscript:msgbox(1))",
        "[x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
        "[x](file:///etc/passwd)",
        "[x](//evil.example/x)",
        "[x](/relative/path)",
        "<javascript:alert(1)>",
        "<data:text/html,<script>alert(1)</script>>",
        "[x][ref]\n\n[ref]: javascript:alert(1)",
        "[**x** <img src=x onerror=alert(1)>](javascript:alert(1))",
        r#"[x](https://example.org "title\" onmouseover=\"alert(1)")"#,
        "[x](https://example.org\" onmouseover=\"alert(1))",
    ];
    for attack in attacks {
        let html = rendered(&app, attack).await;
        assert_only_allowed_markup(&html, attack);
    }
}

#[tokio::test]
async fn supported_markdown_renders() {
    let app = TestApp::new().await;
    let cases = [
        (
            "First paragraph.\n\nSecond paragraph.",
            "<p>First paragraph.</p>\n<p>Second paragraph.</p>",
        ),
        (
            "[the agenda](https://example.org/a?b=1&c=2)",
            r#"<p><a href="https://example.org/a?b=1&amp;c=2" rel="noopener noreferrer nofollow">the agenda</a></p>"#,
        ),
        (
            "<https://example.org>",
            r#"<p><a href="https://example.org" rel="noopener noreferrer nofollow">https://example.org</a></p>"#,
        ),
        (
            "Write to [us](mailto:team@example.org).",
            r#"<p>Write to <a href="mailto:team@example.org" rel="noopener noreferrer nofollow">us</a>.</p>"#,
        ),
        (
            "- Pizza\n- Sushi",
            "<ul>\n<li>Pizza</li>\n<li>Sushi</li>\n</ul>",
        ),
        (
            "1. First\n2. Second",
            "<ol>\n<li>First</li>\n<li>Second</li>\n</ol>",
        ),
        ("Line one  \nLine two", "<p>Line one<br>\nLine two</p>"),
        ("Line one\\\nLine two", "<p>Line one<br>\nLine two</p>"),
        ("Run `make test`.", "<p>Run <code>make test</code>.</p>"),
        ("> Quoted", "<blockquote>\n<p>Quoted</p>\n</blockquote>"),
        ("1 < 2 & 3 > 2", "<p>1 &lt; 2 &amp; 3 &gt; 2</p>"),
    ];
    for (markdown, expected) in cases {
        assert_eq!(
            rendered(&app, markdown).await,
            expected,
            "from {markdown:?}"
        );
    }
}

#[tokio::test]
async fn images_are_dropped_keeping_their_alt_text() {
    let app = TestApp::new().await;

    let html = rendered(&app, "See ![a *cat*](https://tracker.example/cat.png).").await;

    assert_eq!(html, "<p>See a <em>cat</em>.</p>");
    for attack in [
        "![x](javascript:alert(1))",
        "![x\" onerror=\"alert(1)](https://example.org/x.png)",
        "![x](data:image/svg+xml;base64,PHN2Zy8+)",
    ] {
        assert_only_allowed_markup(&rendered(&app, attack).await, attack);
    }
}

#[tokio::test]
async fn headings_render_as_strong_paragraphs() {
    let app = TestApp::new().await;
    let cases = [
        ("# Agenda", "<p><strong>Agenda</strong></p>"),
        (
            "### Agenda *now*",
            "<p><strong>Agenda <em>now</em></strong></p>",
        ),
        ("Agenda\n======", "<p><strong>Agenda</strong></p>"),
    ];
    for (markdown, expected) in cases {
        assert_eq!(
            rendered(&app, markdown).await,
            expected,
            "from {markdown:?}"
        );
    }
}

#[tokio::test]
async fn unsupported_blocks_keep_only_their_text() {
    let app = TestApp::new().await;
    let cases = [
        ("Above\n\n---\n\nBelow", "<p>Above</p>\n\n<p>Below</p>"),
        ("```\nlet x = 1;\n```", "<code>let x = 1;\n</code>"),
        (
            "| a | b |\n|---|---|\n| 1 | 2 |",
            "<p>| a | b |\n|---|---|\n| 1 | 2 |</p>",
        ),
        ("~~struck~~", "<p>~~struck~~</p>"),
    ];
    for (markdown, expected) in cases {
        let html = rendered(&app, markdown).await;
        assert_eq!(html, expected, "from {markdown:?}");
        assert_only_allowed_markup(&html, markdown);
    }
}

#[tokio::test]
async fn html_entities_and_nested_markdown_html_mixes_stay_inert() {
    let app = TestApp::new().await;
    // Entities decode to text, which is escaped again on output.
    let cases = [
        (
            "&lt;script&gt;alert(1)&lt;/script&gt;",
            "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>",
        ),
        (
            "&#60;img src=x onerror=alert(1)&#62;",
            "<p>&lt;img src=x onerror=alert(1)&gt;</p>",
        ),
        (
            "`<script>alert(1)</script>`",
            "<p><code>&lt;script&gt;alert(1)&lt;/script&gt;</code></p>",
        ),
        ("\\<script>alert(1)", "<p>&lt;script&gt;alert(1)</p>"),
    ];
    for (markdown, expected) in cases {
        assert_eq!(
            rendered(&app, markdown).await,
            expected,
            "from {markdown:?}"
        );
    }
    let attacks = [
        "**<script>alert(1)</script>**",
        "*<img src=x onerror=alert(1)>*",
        "[<script>alert(1)</script>](https://example.org)",
        "[x](https://example.org)<script>alert(1)</script>",
        "> <iframe src=https://evil.example></iframe>",
        "- <style>*{}</style>\n- <b onmouseover=alert(1)>x</b>",
        "<scr<script>ipt>alert(1)</script>",
        "<<script>script>alert(1)<</script>/script>",
        "<a href=\"https://example.org\" onclick=\"alert(1)\">**x**</a>",
        "<p>\n\n**inside**\n\n</p>",
        "[x](https://example.org) <a href=javascript:alert(1)>y</a>",
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
        "<form action=https://evil.example><button>Vote</button></form>",
        "<meta http-equiv=refresh content=\"0;url=https://evil.example\">",
        "<base href=https://evil.example/>",
        "<object data=javascript:alert(1)></object>",
        "<details open ontoggle=alert(1)>",
    ];
    for attack in attacks {
        assert_only_allowed_markup(&rendered(&app, attack).await, attack);
    }
}

/// Sends a create request and asserts a 422 naming `description`.
async fn assert_description_rejected(app: &TestApp, description: Value) {
    let admin = app.admin_token().await;
    let shown: String = description.to_string().chars().take(80).collect();
    let request = with_description(description);
    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {shown}"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"], "invalid_input", "{body}");
    assert_eq!(body["field"], "description", "for {shown}: {body}");
}

#[tokio::test]
async fn control_bidi_and_zero_width_characters_are_rejected() {
    let app = TestApp::new().await;
    for hostile in [
        "bell\u{7}",
        "nul\u{0}byte",
        "escape\u{1b}[31m",
        "vertical\u{b}tab",
        "form\u{c}feed",
        "del\u{7f}",
        "c1\u{85}control",
        "bidi\u{202E}override",
        "isolate\u{2066}x\u{2069}",
        "mark\u{200F}",
        "zero\u{200B}width",
        "joiner\u{200D}",
        "bom\u{FEFF}",
        "line\u{2028}separator",
        "paragraph\u{2029}separator",
        "tag\u{E0041}",
    ] {
        assert_description_rejected(&app, json!(hostile)).await;
    }
}

#[tokio::test]
async fn the_description_is_capped_at_10000_characters_after_normalisation() {
    let app = TestApp::new().await;
    // 10 000 x "e + combining acute": 20 000 code points, 10 000 characters
    // after NFC. (Keeps the request under the 64 KiB body limit.)
    let decomposed = "e\u{301}".repeat(10_000);

    let created = app.create_ballot(&with_description(decomposed)).await;

    assert_eq!(
        created["description"].as_str().unwrap().chars().count(),
        10_000
    );
    assert_description_rejected(&app, json!("x".repeat(10_001))).await;
    assert_description_rejected(&app, json!("\u{e9}".repeat(10_001))).await;
}

#[tokio::test]
async fn line_breaks_and_tabs_are_kept_with_crlf_and_cr_normalised_to_lf() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&with_description(
            "  \r\n- one\r\n- two\r\rCafe\u{301}\tbar\n\t ",
        ))
        .await;

    assert_eq!(created["description"], "- one\n- two\n\nCaf\u{e9}\tbar");
}

#[tokio::test]
async fn an_empty_or_non_string_description_is_rejected() {
    let app = TestApp::new().await;
    for invalid in [
        json!(""),
        json!(" \n\t\r\n "),
        json!(null),
        json!(42),
        json!(["a"]),
    ] {
        assert_description_rejected(&app, invalid).await;
    }
}

#[tokio::test]
async fn admin_sets_keeps_and_clears_the_description_by_editing() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    let token = created["token"].as_str().unwrap();

    let set = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "description": "Bring *snacks*.\r\n" }),
        )
        .await;
    assert_eq!(set.status(), StatusCode::OK);
    assert_eq!(body_json(set).await["description"], "Bring *snacks*.");
    assert_eq!(
        app.voter_view(token).await["description"],
        "Bring *snacks*."
    );
    assert_eq!(
        description_html(&page(&app, token).await).trim(),
        "<p>Bring <em>snacks</em>.</p>"
    );

    let kept = app
        .admin_patch(&path, &admin, &json!({ "title": "Dinner?" }))
        .await;
    assert_eq!(body_json(kept).await["description"], "Bring *snacks*.");

    let cleared = app
        .admin_patch(&path, &admin, &json!({ "description": null }))
        .await;
    assert_eq!(cleared.status(), StatusCode::OK);
    assert_eq!(body_json(cleared).await["description"], json!(null));
    assert_eq!(app.voter_view(token).await["description"], json!(null));
    assert_eq!(description_html(&page(&app, token).await), "");
}

#[tokio::test]
async fn an_edit_with_a_hostile_description_is_rejected_and_changes_nothing() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&with_description("Original")).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    for hostile in [
        json!("bidi\u{202E}override"),
        json!("x".repeat(10_001)),
        json!(""),
        json!(42),
    ] {
        let response = app
            .admin_patch(
                &path,
                &admin,
                &json!({ "title": "Changed", "description": hostile }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["field"], "description");
    }
    let ballot = body_json(app.admin_get(&path, &admin).await).await;
    assert_eq!(ballot["description"], "Original");
    assert_eq!(ballot["title"], created["title"]);
}

#[tokio::test]
async fn the_description_shows_on_the_confirmation_step_answered_and_closed_pages() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&with_description("Bring **snacks**."))
        .await;
    let token = created["token"].as_str().unwrap();
    let pizza = created["options"][0]["id"].as_str().unwrap();
    let expected = "<p>Bring <strong>snacks</strong>.</p>";

    let confirm = body_string(
        app.post_form(&format!("/b/{token}"), &format!("option={pizza}"))
            .await,
    )
    .await;
    assert!(confirm.contains("Confirm your answer"), "{confirm}");
    assert_eq!(
        description_html(&confirm).trim(),
        expected,
        "confirmation step"
    );

    app.post_form(
        &format!("/b/{token}"),
        &format!("option={pizza}&confirm=yes"),
    )
    .await;
    let answered = page(&app, token).await;
    assert!(
        answered.contains("Your answer has been recorded"),
        "{answered}"
    );
    assert_eq!(description_html(&answered).trim(), expected, "answered");

    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());
    let closed = page(&app, token).await;
    assert!(closed.contains("Voting closed"), "{closed}");
    assert_eq!(description_html(&closed).trim(), expected, "closed");
}

#[tokio::test]
async fn deeply_nested_markdown_renders_without_failing() {
    let app = TestApp::new().await;
    for pathological in [
        ">".repeat(10_000),
        "- ".repeat(5_000),
        "*".repeat(10_000),
        "[".repeat(5_000) + &"](https://example.org)".repeat(200),
        "<div>".repeat(2_000),
        "`".repeat(10_000),
    ] {
        let created = app.create_ballot(&with_description(pathological)).await;
        let started = std::time::Instant::now();
        let response = app
            .get(&format!("/b/{}", created["token"].as_str().unwrap()))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = description_html(&body_string(response).await).to_owned();
        assert_only_allowed_markup(&html, "pathological input");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
