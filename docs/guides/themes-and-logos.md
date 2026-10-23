# Themes and logos

A **Theme** is a named look for the voting page. Each Ballot uses exactly one Theme, chosen by the admin with `"theme": "<name>"` (default `classic`). On top of it, two **Theme Tweaks** are possible per Ballot: an accent colour (`"accent_color": "#rrggbb"`) and a logo (`"logo": "<name>"`).

This guide is for operators who add Themes and logos, and for Theme authors.

- [Built-in Themes](#built-in-themes)
- [Folder Themes](#folder-themes)
- [Templates and their variables](#templates-and-their-variables)
- [CSS custom properties](#css-custom-properties)
- [The voting page's script](#the-voting-pages-script)
- [The hidden revision field](#the-hidden-revision-field)
- [Content Security Policy rules for Themes](#content-security-policy-rules-for-themes)
- [Logos](#logos)
- [Folder permissions](#folder-permissions)
- [Startup checks and errors](#startup-checks-and-errors)
- [Caching](#caching)
- [Worked example: the `acme` Theme](#worked-example-the-acme-theme)

## Built-in Themes

The binary ships three Themes. Each is a stylesheet on top of the built-in templates (`templates/` in the repository):

| Name      | Look |
| --------- | ---- |
| `classic` | The default: a calm card on a soft grey-blue background, Options as large cards. Light, or dark when the voter's system prefers dark (`prefers-color-scheme`). It is also the fallback when a Ballot's Theme no longer exists. |
| `dark`    | The same layout, always dark (`color-scheme: dark`), with an accent line on the card. |
| `minimal` | Plain serif typography, no card, Options as rows between hairline rules; black on white, or white on near-black when the system prefers dark. |

None of them loads fonts or images from anywhere. All three style the same built-in templates and the script's states (see [The built-in templates' structure](#the-built-in-templates-structure)), honour `prefers-reduced-motion` and `forced-colors`, and keep text at WCAG AA contrast.

## Folder Themes

Set [`theme_folder`](../reference/configuration.md#theme_folder) to add Themes without rebuilding. Each subdirectory is one Theme, named after the directory:

```text
<theme_folder>/
└── acme/                    ← Theme name "acme"
    ├── style.css            ← required
    └── templates/           ← optional
        ├── base.html        ← optional, each one replaces the built-in of that name
        ├── ballot.html
        └── confirm.html
```

Rules:

- **Names**: `^[a-z0-9][a-z0-9-]{0,63}$`: lowercase ASCII letters, digits and `-`, starting with a letter or digit, at most 64 characters. A directory with any other name (e.g. `Bad-Name`) is ignored with a warning in the log. Files directly in the theme folder are ignored with a warning as well.
- **`style.css` is required.** It is served as-is at `/assets/themes/<name>/style.css`.
- **Templates are optional.** Any of `base.html`, `ballot.html` and `confirm.html` may be overridden. The rest come from the built-ins, so a Theme that overrides only `base.html` gets the built-in `ballot.html` extending *its* `base.html`. Other files in `templates/` are ignored with a warning.
- **Overriding a built-in**: a folder Theme with the name of a built-in Theme (e.g. `classic`) replaces it entirely: its stylesheet and its templates. The log says `folder Theme "classic" overrides the built-in one`.
- **Symlinks** are followed, so a Theme may be linked in from elsewhere. For the same reason **the theme folder must be writable by root only** (see [Folder permissions](#folder-permissions)).
- **Loaded once at startup.** Restart the server to pick up changes. The admin API rejects unknown Theme names, so a Theme must exist before Ballots can use it.
- **Removing a Theme** that Ballots still use is allowed. Their voting pages fall back to `classic`. The admin view still shows the stored name, and the voter API shows `"theme": "classic"`. An admin edit that doesn't touch `theme` doesn't re-check it.

Request-supplied names are only ever looked up among the registry's known names. They are never joined onto a file path.

## Templates and their variables

Templates are [MiniJinja](https://docs.rs/minijinja/) (Jinja2 syntax), auto-escaped as HTML. `ballot.html` and `confirm.html` extend `base.html` and fill its blocks `title` and `content`. Only MiniJinja's built-in filters and tests are available. Undefined variables are lenient: they render as nothing and are falsy, so `{% if some_new_variable %}` is safe.

The authoritative list is the module comment in `src/web/templates.rs`. Every variable below is present on every page of its template; "none" means it is undefined/`none` and falsy.

### Every template (`base.html`, `ballot.html`, `confirm.html`)

| Variable           | Value |
| ------------------ | ----- |
| `theme`            | Name of the Theme in use. |
| `base_path`        | The [base path](../reference/configuration.md#public_base_url) every URL of the app starts with: `/` at the root of a host, or e.g. `/polls/` under a [sub-path](deployment.md#under-a-sub-path). Always with a leading and a trailing slash; only ASCII letters, digits, `-`, `.`, `_`, `~` and `/`, so it is inserted unescaped. |
| `theme_stylesheet` | Root-relative URL of the Theme's stylesheet, `<base_path>assets/themes/<theme>/style.css` (e.g. `/assets/themes/classic/style.css`). **`base.html` must link it.** |
| `accent_color`     | `#rrggbb` (lowercase) if the Ballot has an accent colour, else none. |
| `accent_contrast`  | `#000000` or `#ffffff`, whichever has the higher WCAG contrast on `accent_color`. None without an accent colour. |
| `logo_url`         | Root-relative URL of the Ballot's logo (`<base_path>assets/logos/<name>`), or none (no logo, or the logo has been removed). |
| `csp_nonce`        | This page's style nonce, for `<style nonce="{{ csp_nonce }}">`. |
| `title`            | The Ballot's title (plain text, escaped). |
| `description`      | The description rendered to sanitised HTML (inserted unescaped), or none. Safe **as element content only** (see below). |

**Put `description` only between tags** of an ordinary element, as the built-in templates do: `<div class="description">{{ description }}</div>`. The sanitised HTML is safe there, but it is not escaped for any other place: its text may contain `"` and `'`, so inside an attribute value (`title="{{ description }}"`, `content="{{ description }}"`) it could end the attribute and add attributes of the admin's choosing. It doesn't belong in `<title>`, `<textarea>`, `<script>` or `<style>` either, where it is not read as markup. To show the description in such a place, write a fixed text there instead.

**Never write the app's URLs as `/…` in a template**: on a sub-path deployment they would miss the prefix. Use `theme_stylesheet` and `logo_url` as they are, and build any other URL of the app from `base_path`, e.g. a logo by name regardless of the Ballot's logo Tweak: `<img src="{{ base_path }}assets/logos/acme" alt="ACME">`. The built-in templates need nothing else: their forms post to the page itself (`action=""`) and the confirmation page's "go back" link is `href=""`; keep it that way in your own.

### `ballot.html` (the voting page, in every state)

| Variable            | Value |
| ------------------- | ----- |
| `options`           | All Options, each with `id`, `label` and `checked` (in the current Response; without one, while voting is open, in the Preselection). |
| `multiple`          | `true` on a multiple-choice Ballot (checkboxes), `false` on single choice (radio buttons). |
| `allow_no_option`   | `true` if the voter may answer with no Option at all (Allow No Option, multiple choice only). The built-in legend then reads "Choose any number of options (or none)". |
| `editable`          | `true` if the Submission Mode is Editable. |
| `answered`          | `true` if the Ballot has a Response. |
| `preselected`       | `true` if the ticked Options are the admin's Preselection, not the voter's choice: on a Final Ballot without a Response while voting is open (the built-in template then says "Preselected; confirm or change, then submit."), or on an Editable Ballot whose Response is still the Preselection, which the voter hasn't saved (then `answered` is `true` too; the built-in template says "Preselected for you. You can change it until voting closes." and, once closed, "You kept the preselection, so this was recorded:"). |
| `read_only`         | `true` if the Response is shown without a form (Final and answered, or closed with a Response). |
| `closed`            | `true` once the Voting Deadline has passed. |
| `selected`          | Options of the current Response (`id`, `label`); empty without one, and also for a Response of no Option (tell them apart by `answered`; the built-in template then says "You chose no option."). |
| `upcoming_fallback` | While voting is open and there is no Response: the Fallback's Options (`id`, `label`), else none. |
| `fallback`          | Once voting has closed without a Response: the applied Fallback's Options, else none. |
| `message`           | A notice for the voter (why a posted Response was refused or invalid), or none. Show it, e.g. with `role="alert"`. |
| `deadline_utc`      | The Voting Deadline as RFC 3339 UTC, for `<time datetime="…">`. |
| `deadline_local`    | The deadline formatted in [`default_time_zone`](../reference/configuration.md#default_time_zone), short, with the year and zone, e.g. `Tue 1 Dec 2026, 13:00 CET`. (With JavaScript the script shows it in the voter's own zone instead, e.g. `Tomorrow, 13:00`.) |
| `status`            | The Ballot Status in words: `Unanswered`, `Answered` or `Auto-fallback`. |
| `revision`          | The Ballot's revision (an integer). Not needed: the app [inserts the hidden field](#the-hidden-revision-field) itself. |

### `confirm.html` (the Final Ballot's confirmation step, no-JS only)

| Variable   | Value |
| ---------- | ----- |
| `selected` | The Options the voter chose (`id`, `label`); empty if they chose none on a Ballot with Allow No Option (the built-in template then says "You chose no option." and posts no `option` field). |
| `revision` | The Ballot's revision. Not needed: the app [inserts the hidden field](#the-hidden-revision-field) itself. |

`confirm.html` must post them again with `confirm=yes`, in a `<form method="post">` (the app adds the revision):

```html
<form method="post" action="">
  {% for option in selected %}
  <input type="hidden" name="option" value="{{ option.id }}">
  {% endfor %}
  <button type="submit" name="confirm" value="yes">Confirm and submit</button>
</form>
```

### What `ballot.html` must keep

Without JavaScript the page is a plain form, and the script builds on the same markup. A `ballot.html` must keep:

- a `<form method="post" action="">` (posting to the page itself) while the Ballot takes a Response; the app adds the hidden `revision` field to it ([below](#the-hidden-revision-field)), and the [startup check](#startup-checks-and-errors) fails without it;
- inputs named `option` with the Option `id` as value: `type="radio"` when not `multiple`, `type="checkbox"` when `multiple`, each with an `id` and a `<label for="…">`;
- the deadline as `<time datetime="{{ deadline_utc }}">{{ deadline_local }}</time>`, so the script can show it in local time;
- everything that changes with the Ballot Status inside `<body>`: the script saves without a reload by swapping in the server's fresh `<body>`.

Optional, for the script:

- a **countdown placeholder**: an empty element with the class `vota-countdown`, e.g. `<span class="vota-countdown" role="timer"></span>` beside the deadline. The script fills it ("in 2 days, 4 hours", at the deadline "Voting has closed."). It no longer adds a countdown of its own: without a placeholder the page shows none, but still closes the form at the deadline and reloads;
- a **card per Option**: the input and its label inside an element with the class `option` (the built-in `<div class="option">`). On an Editable Ballot the script marks that element's save state (`data-vota-save`, see [below](#the-voting-pages-script)); without one it marks the input's parent element.

Start from the built-in `templates/ballot.html` and change it rather than writing one from scratch.

### The built-in templates' structure

The built-in Themes style these class names; a Theme that keeps the built-in templates (only a `style.css`) can rely on them:

| Class | Element |
| ----- | ------- |
| `ballot-header` | The `<header>` with the title (`h1`) and `.meta`. |
| `meta` | The row under the title: `.status`, `.deadline` (with `<time>` and the `.vota-countdown` placeholder) and, on its own line, `.submission-mode` or `.recorded`. |
| `status` | The status pill: `data-state="open"`, `"answered"` or `"closed"`, its text `Open`, `Answered` or `Closed` (after a visually hidden "Status: "), the Ballot Status in its `title`. |
| `submission-mode` | "You can change your answer until then." (Editable, in `.meta`); with `final`: "Once you submit your answer, you cannot change it." (Final, in `.actions` beside the button). |
| `recorded` | "Your answer has been recorded." (Final, answered), or with `submission-mode` "Your answer is saved. You can change it until then." (Editable, answered); with `preselected` too, "Preselected for you. You can change it until voting closes." (Editable, answered by the Preselection). |
| `description`, `message` | The description; a notice why a post was refused (`role="alert"`). |
| `fallback` | While open with a Fallback: the hint "No answer by the deadline? The Option marked default will be recorded." (`id="fallback-hint"`, which the Fallback's inputs name in `aria-describedby`). |
| `preselection-hint` | On a Final Ballot with a Preselection, while open and unanswered: the hint "Preselected; confirm or change, then submit." above the Options. |
| `fallback-badge` | The small "default" badge on each Fallback Option's card (and in the hint). |
| `outcome` | Read-only and closed views: one line, `p.chosen` or `p.closed` ("You chose", "You kept the preselection, so this was recorded:", "No answer was given, so this was recorded:", "No answer was given.") and the `ul.selection` chips (`fallback-applied` when the Fallback was recorded). |
| `options`, `option` | The Option cards inside the `fieldset`; each `.option` holds the input, its `label` and maybe a `.fallback-badge`. |
| `actions` | The submit button's row (and the script's status line on an Editable Ballot). |
| `visually-hidden` | Text for screen readers only (the built-in Themes define it). |

`confirm.html` uses `ballot-header`, `confirmation` (around its `h2`, `.outcome`, `.warning` and the form), `actions` (the form) and `back` (the "Go back and choose again" link).

Earlier versions showed the Fallback as `p.fallback` plus `ul.fallback-options`, the Ballot Status as `<p class="status">Status: …</p>`, and the outcome as a sentence plus a bulleted `ul.selection`. Folder Themes with their own templates are unaffected; a stylesheet-only Theme written for the old markup should be checked against the classes above.

## CSS custom properties

The accent colour Theme Tweak works through CSS custom properties. The built-in `base.html` sets them in a nonce'd style element when the Ballot has an accent colour:

```html
{% if accent_color %}
<style nonce="{{ csp_nonce }}">:root { --accent: {{ accent_color }}; --accent-contrast: {{ accent_contrast }}; }</style>
{% endif %}
```

A Theme's stylesheet should therefore define its own defaults on `:root` and use them everywhere:

| Property            | Used for | Set by the Theme Tweak |
| ------------------- | -------- | ---------------------- |
| `--accent`          | Buttons, the selected Option, highlights | yes |
| `--accent-contrast` | Text on `--accent` (button labels) | yes |
| `--bg`              | Page background; dialog background fallback | no |
| `--surface`         | Card and dialog background; text on the state icons | no |
| `--text`            | Text colour | no |
| `--muted`           | Secondary text: the meta line, the idle status line, badges | no |
| `--border`          | Borders | no |
| `--danger`, `--danger-bg` | Error notices and dialogs; a failed save (Option card, status line) | no |
| `--success`, `--success-bg` | A saved change: the status line, the badge, the fading highlight | no |
| `--radius`, `--radius-sm` | Corner radius of the card and dialog; of the status line and small boxes | no |

The script's stylesheet (`/assets/app/ballot.css`) uses all of these but `--accent-contrast`, each with a fallback, so a Theme may define any subset, and defining them is how a Theme restyles the save states. The built-in Themes also define some extras for their own use, such as `--surface-sunken`, `--border-strong`, `--selected-bg`, `--notice-bg`, `--link`, `--focus`, a spacing scale (`--space-1` … `--space-7`) and a type scale (`--step--1` … `--step-3`). Don't use `--accent` for body text or links: any colour can be chosen as accent, and contrast would not hold.

An accent colour is only ever applied as the value of `--accent`, never as raw CSS, and only after it has been validated as `#rrggbb`.

## The voting page's script

Templates don't include the app's script. The app inserts it and its stylesheet into every rendered `ballot.html` page, right before the first `</head>` (without one, before `</body>`; without either, at the end), with every URL under the base path (here `/`; under a sub-path e.g. `/polls/assets/app/<version>/main.js` and `data-api="/polls/api/b/…"`):

```html
<link rel="stylesheet" href="/assets/app/ballot.css?v=ba35bbdb81344a06">
<script type="module" src="/assets/app/0d4c1e7a9b2f6835/main.js" id="vota-enhancement" data-api="…" data-page="…" data-deadline="…" data-now="…" data-open="true" data-revision="0" data-choice-mode="single" data-submission-mode="final" data-needs-confirmation="true" data-allow-no-option="false" data-answered="false" data-preselected="false"></script>
```

So a Theme can't drop it. The script is a set of native ES modules (`main.js` imports its siblings relatively); it finds its own tag by the id `vota-enhancement`, so a Theme must not give that id to anything else. Browsers without module support get the plain no-JS page. The stylesheet comes after the Theme's stylesheet, but every styling rule is wrapped in `:where()` (zero specificity), so a Theme's own rules win; only the rules hiding the autosaved form's Submit button and an idle "Retry now" use `!important`.

What the script adds or sets, all of which a Theme may restyle:

| Hook | What |
| ---- | ---- |
| `.vota-countdown` | The template's countdown placeholder, filled with "in 2 days, 4 hours" (a timer, not announced on every tick). |
| `form[data-vota-autosave]` | An Editable Ballot's form while it autosaves; its Submit button gets `hidden`, except while a Ballot with Allow No Option has no Response yet (Save then saves "no Option"). |
| `.vota-autosave` with `data-state` | The status line, put where the Submit button was: `idle` ("Changes are saved automatically."), `pending` ("Saving…"), `saved` ("Saved"), `failed` ("Not saved, retrying…" or "Not saved." with the reason) or `invalid` (an empty multiple-choice selection without Allow No Option, not sent). Inside: `.vota-autosave-status` (`role="status"`, `aria-live="polite"`) with `.vota-autosave-icon`, `.vota-autosave-text` and `.vota-autosave-reason`, and the `button.vota-retry` ("Retry now", only while failed). |
| `data-vota-save` on an Option's card | `pending`, `saved` (removed after about two seconds; `ballot.css` fades a highlight from `--success-bg`) or `failed` (until resolved), only on Options whose shown state differs from what is stored (or was just saved). The card is the input's closest `.option`, else its parent. |
| `.vota-option-state` | The badge appended to such a card: "Saving…", "Saved" or "Not saved" (`aria-hidden`; the status line says it in words). |
| `.vota-dialog` with `data-state` | The dialog: `confirm`, `saving`, `saved` or `error`; inside `.vota-dialog-title`, `.vota-dialog-message`, `.vota-selection`, `.vota-warning`, `.vota-dialog-actions` with `button.vota-primary` and `button.vota-secondary`. |
| `.vota-visually-hidden` | The live region announcing "Voting has closed." |
| `time[data-vota-local]` | A `<time>` rewritten into the voter's local time; its `title` holds the precise date and zone. |

The built-in Themes style the failed card themselves (`.options .option[data-vota-save="failed"]`, specific enough to beat their selected-card rule); `ballot.css` only outlines it in `--danger` for Themes that don't. The dialog's and status line's buttons are plain `<button>`s and get the Theme's button styles; give `.vota-secondary` an outlined look.

`confirm.html` gets no script: with JavaScript, confirmation happens in a dialog on the voting page instead.

## The hidden revision field

A Response is only recorded against the Ballot as its page showed it: every no-JS post carries the Ballot's revision, and one without it is refused (`422`, "The page was out of date or incomplete."). Templates don't write that field. After rendering `ballot.html` and `confirm.html`, the app inserts

```html
<input type="hidden" name="revision" value="3">
```

(with the current revision) right after the start tag of every `<form method="post">` on the page, the same way it [inserts the script](#the-voting-pages-script). So a Theme can't forget it, and each form ends up with exactly one:

- A form that already has an `<input>` named `revision` is left as it is. Themes written for earlier versions, which had to write `<input type="hidden" name="revision" value="{{ revision }}">` themselves, keep working unchanged; you may delete that line from them.
- Only an `<input>` may be named `revision`. A `<select>`, `<textarea>` or `<button>` named `revision` in a `<form method="post">` could make the form post `revision` twice (a `422`), so the [startup check](#startup-checks-and-errors) refuses such a Theme. Outside a POST form (in a `method="get"` form, say) such a control is left alone.
- Ordinary variations are fine: tag and attribute names in any case (`<FORM METHOD=POST>`), double, single or no quotes, any attribute order, whitespace and newlines inside the tag, a `>` inside a quoted attribute value, several forms on a page.
- Only real form tags get the field. Nothing is inserted into comments, attribute values, or the text of `<script>`, `<style>`, `<textarea>`, `<title>` (and the rarer `xmp`, `iframe`, `noembed`, `noframes`). Forms with `method="get"` or without `method` get nothing. A `<form>` nested in another form is ignored, as browsers ignore it.

Limits, which ordinary templates never run into: character references in attribute values are not decoded (`method="&#112;ost"` is not seen as POST); an `<input name="revision">` counts even if it is `disabled` or belongs to another form via `form="…"`; `<noscript>` content is treated as markup, as a browser without JavaScript reads it; and a `<form>` inside `<template>` or SVG/MathML is treated like any other.

## Content Security Policy rules for Themes

Voting pages are served with a strict [Content Security Policy](../concepts/security-headers.md). For Theme authors this means:

- **No inline scripts** and no event-handler attributes (`onclick`, `onload`, …). They are blocked. The app's own script is the only script.
- **No `style="…"` attributes.** They are blocked. Put styles in `style.css`, or in a `<style nonce="{{ csp_nonce }}">` element in a template.
- **Nothing from other hosts**: no web fonts, CDN stylesheets, remote images or analytics. Only same-origin resources load. Use system fonts. `img-src 'self'` also blocks `data:` images, so put images in the logo folder.
- **`@import` and `url()` in `style.css`** may only point at same-origin URLs. The app serves only `style.css` itself from a Theme folder, so a Theme cannot ship extra files such as fonts or background images.
- **Forms** may only post to the same origin.

## Logos

Set [`logo_folder`](../reference/configuration.md#logo_folder). Every file named `<name>.svg`, `<name>.png` or `<name>.webp` is a logo called `<name>`. Any logo can be combined with any Theme.

- **Names** follow the Theme name rule (`^[a-z0-9][a-z0-9-]{0,63}$`). Extensions are exact and lowercase. Other files (`Logo.svg`, `logo.jpg`, `notes.txt`) are ignored with a warning.
- **One file per name**: `acme.svg` *and* `acme.png` is a startup error.
- **At most 512 KiB** per file.
- **Contents must match the extension**: PNG signature, a RIFF/WEBP container, or UTF-8 text containing `<svg`. Otherwise startup fails.
- Served at `<base_path>assets/logos/<name>` (no extension) with the matching `Content-Type`. Pages show it as `<img class="logo" src="{{ logo_url }}">`. The voter API gives the absolute `logo_url`.
- Loaded once at startup. A logo removed while Ballots use it simply isn't shown any more.
- An SVG logo is served with `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`, so a logo opened directly runs no scripts, loads nothing and is an opaque origin. PNG and WebP logos get `default-src 'none'`. Pages embed logos only through `<img>`, where SVG never runs scripts.
- **Symlinks** are followed, as in the theme folder, so **the logo folder must be writable by root only** too.

## Folder permissions

The app follows symlinks in the theme and logo folders, and it serves `style.css` and logos to anyone, without a Ballot Token. So whoever can create or replace a file in either folder can link it to any file the service can read, and publish that file. For example, `style.css` linked to the database would be served at `/assets/themes/<name>/style.css` with every live Ballot Token in it. Vota doesn't refuse symlinks, because linking a Theme in from elsewhere is a legitimate use; the folders must be trusted instead:

- **Both folders, everything in them, and the directories above them are writable by root only.** The service only needs to read them. Don't give the service user, a deploy user or a group write access.
- Files linked in from elsewhere are subject to the same rule: their own directories must be writable by root only.

For example:

```sh
chown -R root:root /etc/vota/themes /etc/vota/logos
chmod -R u=rwX,go=rX /etc/vota/themes /etc/vota/logos
```

On NixOS, a folder given as a Nix path (`theme_folder = ./themes;`) is copied to the read-only Nix store, which already meets this. A folder given as a string (`"/etc/vota/themes"`) needs the permissions above, and must be readable by any user and not lie under `/home`, `/root` or `/run/user`.

## Startup checks and errors

When `serve` starts (the `admin-token` commands don't read Themes or logos), each Theme is loaded and **trial-rendered**: `ballot.html` in every voting page state (open single/multiple choice, Final/Editable, answered, answered with no Option, closed with a Response, with one of no Option, with the Fallback applied, without an answer) and `confirm.html` for both Choice Modes and with no Option chosen, each with and without description, logo, accent colour and notice, and at the base path `/` as well as under a sample prefix (`/sample/base-path/`), so a Theme that only breaks under a sub-path fails on a root deployment too. The sample data is obviously fake. A template that parses but fails when rendered (an `extends` of a missing template, an unknown filter, …) stops startup instead of giving voters an error page later. So does a Theme whose voting page, in a state that takes a Response (open and unanswered, or Editable), or whose confirmation step has no `<form method="post">` for the app to put the [hidden revision field](#the-hidden-revision-field) in, and so does a Theme with a `<select>`, `<textarea>` or `<button>` named `revision` in a `<form method="post">` of either page, in any sample state. If the form went missing through an overridden `base.html` (say, one without `{% block content %}`), the error names the built-in template and that `base.html`. The error names the Theme, the file and the sample page. Real examples:

```text
Error: the Theme "broken" does not render: template themes/broken/templates/ballot.html fails on the sample page (open, single choice, Final, without description, logo, accent colour or notice, at base path /): unknown filter: filter shout is unknown (in ballot.html:2)

Error: the Theme "formless" has no form to answer with: template themes/formless/templates/ballot.html renders the sample page (open, single choice, Final, without description, logo, accent colour or notice, at base path /) without a <form method="post"> the app can add the hidden revision field to

Error: the Theme "mine" has a <button> named "revision": template themes/mine/templates/ballot.html renders the sample page (open, single choice, Final, without description, logo, accent colour or notice, at base path /) with it in a <form method="post">, which the app adds the hidden revision field to; only an <input> may be named "revision"

Error: reading the Theme stylesheet themes/nocss/style.css

Caused by:
    No such file or directory (os error 2)

Error: the logo logos/oops.png is not a valid png file

Error: reading the theme folder /nonexistent

Caused by:
    No such file or directory (os error 2)
```

Other startup errors: a template that doesn't parse (`the Theme template … does not parse: …`), two files for one logo name (`two logo files for the name "acme": …; keep one`), and a logo over 512 KiB (`the logo … is larger than 512 KiB`). Ignored entries only log a `WARN`:

```text
WARN vota::web::themes: ignoring themes/Bad-Name: a Theme name consists of lowercase letters, digits and '-' (starting with a letter or digit, at most 64)
```

If a template still fails at request time (it shouldn't, after the trial render), the voter gets the neutral "Something went wrong" page (`500`), and the error is logged without Ballot Tokens or Voter References.

## Caching

Theme stylesheets and logos are served with `Cache-Control: public, max-age=3600` and have no versioned URLs. After you change a stylesheet or logo and restart, browsers may keep the old file for up to an hour. If a change must show at once, publish it under a new Theme or logo name and point the Ballots at it. The app's own script and stylesheet are referenced by content version (the script's modules under `/assets/app/<version>/`, the stylesheet as `ballot.css?v=<version>`) and cached for a year under it; a request with another version gets the one-hour cache.

## Worked example: the `acme` Theme

This example was created in a temporary directory and checked against a real `vota serve`. It overrides only `base.html`, to add a footer and alt text for the logo. `ballot.html` and `confirm.html` stay the built-ins.

### 1. Files

```text
themes/acme/style.css
themes/acme/templates/base.html
logos/acme.svg
```

`themes/acme/style.css`:

```css
/* Theme "acme": the built-in templates with a warm, rounded look. */
:root {
  --accent: #b4441b;
  --accent-contrast: #ffffff;
  --bg: #fbf6f1;
  --surface: #ffffff;
  --text: #2b1d16;
  --border: #e3d3c6;
  --danger: #a4161a;
  --radius: 1rem;
}
body { margin: 0; background: var(--bg); color: var(--text); font-family: system-ui, sans-serif; line-height: 1.5; }
.masthead, main, .site-footer { max-width: 40rem; margin: 0 auto; padding: 1rem; }
main { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
.logo { max-height: 3rem; }
.message { border-left: 0.3rem solid var(--danger); padding: 0.5rem 1rem; }
.option { margin: 0.5rem 0; }
button { background: var(--accent); color: var(--accent-contrast); border: 0; border-radius: var(--radius); padding: 0.6rem 1.4rem; font: inherit; cursor: pointer; }
.site-footer { color: #6b5a50; font-size: 0.875rem; }
```

`themes/acme/templates/base.html` (the built-in one, plus ` · ACME` in the title, alt text for the logo and a footer):

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex, nofollow">
<title>{% block title %}{% endblock %} · ACME</title>
<link rel="stylesheet" href="{{ theme_stylesheet }}">
{% if accent_color %}
<style nonce="{{ csp_nonce }}">:root { --accent: {{ accent_color }}; --accent-contrast: {{ accent_contrast }}; }</style>
{% endif %}
</head>
<body>
{% if logo_url %}
<header class="masthead"><img class="logo" src="{{ logo_url }}" alt="ACME"></header>
{% endif %}
<main>
{% block content %}{% endblock %}
</main>
<footer class="site-footer">Questions? Ask the ACME works council.</footer>
</body>
</html>
```

`logos/acme.svg`:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 32"><rect width="120" height="32" rx="6" fill="#b4441b"/><text x="60" y="22" font-family="sans-serif" font-size="16" fill="#fff" text-anchor="middle">ACME</text></svg>
```

### 2. Configuration and start

`vota.toml`:

```toml
database_path = "vota.db"
public_base_url = "http://localhost:8080/"
theme_folder = "themes"
logo_folder = "logos"
```

```sh
export VOTA_CONFIG=vota.toml
TOKEN=$(vota admin-token create 2>/dev/null)
vota serve
```

The log lists the new Theme and logo:

```text
INFO vota::web::themes: Themes: acme, classic, dark, minimal; logos: acme
INFO vota::cli::serve: listening on 127.0.0.1:8080
```

### 3. A Ballot using it

```sh
curl -s -X POST http://localhost:8080/admin/ballots \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title": "Canteen menu", "options": [{"label": "Keep it"}, {"label": "Change it"}], "voting_deadline": "2026-12-01T12:00:00Z", "theme": "acme", "logo": "acme", "accent_color": "#1f6f8b"}'
```

The response contains `"theme":"acme"`, `"logo":"acme"`, `"accent_color":"#1f6f8b"` and `"url":"http://localhost:8080/b/-hOo6wzrXM9ygkWznaPMCA"`.

### 4. What the voter gets

The head of the page shows the Theme's stylesheet, the accent colour Tweak in a nonce'd style element, the app's script inserted before `</head>`, and the logo:

```html
<title>Canteen menu · ACME</title>
<link rel="stylesheet" href="/assets/themes/acme/style.css">

<style nonce="Feyo432_Die-QTT4EiNgNg">:root { --accent: #1f6f8b; --accent-contrast: #ffffff; }</style>

<link rel="stylesheet" href="/assets/app/ballot.css?v=ba35bbdb81344a06">
<script type="module" src="/assets/app/0d4c1e7a9b2f6835/main.js" id="vota-enhancement" data-api="/api/b/-hOo6wzrXM9ygkWznaPMCA/response" data-page="/b/-hOo6wzrXM9ygkWznaPMCA" data-deadline="2026-12-01T12:00:00Z" data-now="2026-10-17T10:52:53.567077535Z" data-open="true" data-revision="0" data-choice-mode="single" data-submission-mode="final" data-needs-confirmation="true" data-allow-no-option="false" data-answered="false" data-preselected="false"></script>
</head>
<body>

<header class="masthead"><img class="logo" src="/assets/logos/acme" alt="ACME"></header>
```

…and the footer after `</main>`:

```html
</main>
<footer class="site-footer">Questions? Ask the ACME works council.</footer>
```

The voter API reports the same look:

```sh
curl -s http://localhost:8080/api/b/-hOo6wzrXM9ygkWznaPMCA
# … "accent_color":"#1f6f8b" … "logo_url":"http://localhost:8080/assets/logos/acme" … "theme":"acme" …
```

The assets are served from the app itself:

```sh
curl -s -I http://localhost:8080/assets/logos/acme
# HTTP/1.1 200 OK
# content-type: image/svg+xml
# x-content-type-options: nosniff
# cache-control: public, max-age=3600
# content-security-policy: default-src 'none'; style-src 'unsafe-inline'; sandbox
```

### Variations

- **Restyle a built-in Theme**: create `<theme_folder>/classic/style.css` (copy `themes/classic/style.css` from the repository and edit it). All Ballots using `classic`, and those whose Theme has gone, get it.
- **Change only the voting page**: put just `templates/ballot.html` in the Theme. It extends the built-in `base.html`.
- **Under a sub-path**: with `public_base_url = "http://localhost:8080/polls/"` the same files work unchanged. The page then links `/polls/assets/themes/acme/style.css`, `/polls/assets/app/<version>/main.js` (with `data-api="/polls/api/b/<token>/response"` and `data-page="/polls/b/<token>"`) and `<img class="logo" src="/polls/assets/logos/acme" alt="ACME">`; the admin API returns `"url":"http://localhost:8080/polls/b/<token>"` and the voter API `"logo_url":"http://localhost:8080/polls/assets/logos/acme"`.
- **A fixed logo, whatever the Ballot's Tweak**: in `base.html`, `<header class="masthead"><img class="logo" src="{{ base_path }}assets/logos/acme" alt="ACME"></header>` renders as `src="/assets/logos/acme"` at the root and `src="/polls/assets/logos/acme"` under `/polls/`.
- **Check a Theme before deploying**: start `serve` on the same configuration with a throwaway database and any free port, e.g. `VOTA_DATABASE_PATH=/tmp/theme-check.db VOTA_PORT=0 vota --config <config> serve`. It loads and trial-renders all Themes and fails before `listening on …` if one is broken; otherwise stop it with Ctrl-C. `admin-token` commands don't load Themes, so they can't be used for this check.
