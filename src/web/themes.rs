//! The Theme registry: which Themes exist, by name, and what they consist of.
//!
//! Built-in Themes ([`BUILT_IN`]) ship with the binary; each is a stylesheet
//! on top of the built-in templates. Folder Themes are loaded from the theme
//! folder once, at startup (a restart picks up changes), and a folder Theme
//! with a built-in Theme's name replaces it entirely.
//!
//! Names are only ever looked up among the registry's keys; a name from a
//! request is never joined onto a path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use axum::body::Bytes;
use minijinja::{Environment, Value};

use super::logos::{self, Logo};
use super::templates;
use super::trial_render;
use crate::domain::ballot::Ballot;
use crate::domain::theme::{DEFAULT_THEME, ThemeCatalog, is_valid_name};

/// Built-in Themes: name and stylesheet.
const BUILT_IN: [(&str, &str); 3] = [
    ("classic", include_str!("../../themes/classic/style.css")),
    ("dark", include_str!("../../themes/dark/style.css")),
    ("minimal", include_str!("../../themes/minimal/style.css")),
];

/// One Theme: its stylesheet and its templates.
pub(crate) struct Theme {
    stylesheet: Bytes,
    templates: Environment<'static>,
}

impl Theme {
    pub(crate) fn stylesheet(&self) -> &Bytes {
        &self.stylesheet
    }
}

/// The Theme a Ballot is shown in and the logo it shows: the Ballot's, as
/// far as they (still) exist ([`Themes::look`]).
pub(crate) struct Look<'a> {
    /// The name of the Theme in use.
    pub(crate) theme: &'a str,
    /// The logo's name, if the Ballot has one that exists.
    pub(crate) logo: Option<&'a str>,
    /// What every URL starts with, ending in `/`: the base path (`/`, or
    /// e.g. `/polls/`) for root-relative URLs, or the public base URL for
    /// absolute ones.
    pub(crate) base: &'a str,
}

impl Look<'_> {
    /// The logo's URL under the base, if there is a logo.
    pub(crate) fn logo_url(&self) -> Option<String> {
        let base = self.base;
        self.logo.map(|name| format!("{base}assets/logos/{name}"))
    }

    /// The Theme's stylesheet URL under the base.
    pub(crate) fn theme_stylesheet_url(&self) -> String {
        let Self { base, theme, .. } = self;
        format!("{base}assets/themes/{theme}/style.css")
    }
}

/// The Theme registry: Themes and logos by name.
pub(crate) struct Themes {
    themes: BTreeMap<String, Theme>,
    logos: BTreeMap<String, Logo>,
}

impl Themes {
    /// The built-in Themes, overridden and extended by those in
    /// `theme_folder` (if any), and the logos in `logo_folder` (if any).
    /// Fails with an error naming the file at fault.
    pub(crate) fn load(
        theme_folder: Option<&Path>,
        logo_folder: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let mut themes = BTreeMap::new();
        for (name, stylesheet) in BUILT_IN {
            let templates = templates::environment(Vec::new())
                .map_err(|(name, error)| anyhow::anyhow!("built-in template {name}: {error}"))?;
            trial_render::check(name, &templates).map_err(|failure| {
                trial_failure(name, &format!("built-in {}", failure.template), &failure)
            })?;
            let theme = Theme {
                stylesheet: Bytes::from_static(stylesheet.as_bytes()),
                templates,
            };
            themes.insert(name.to_owned(), theme);
        }
        if let Some(folder) = theme_folder {
            for (name, dir) in theme_dirs(folder)? {
                let theme = load_folder_theme(&name, &dir)?;
                if themes.insert(name.clone(), theme).is_some() {
                    tracing::info!("folder Theme {name:?} overrides the built-in one");
                }
            }
        }
        let logos = match logo_folder {
            Some(folder) => logos::load(folder)?,
            None => BTreeMap::new(),
        };
        tracing::info!(
            "Themes: {}; logos: {}",
            themes.keys().cloned().collect::<Vec<_>>().join(", "),
            logos.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        Ok(Self { themes, logos })
    }

    /// The Ballot's Theme and logo as the registry has them now, for URLs
    /// under `base`: the one resolution the voting page and the voter API
    /// share, so they can't disagree. A Theme removed from the theme folder
    /// since is the default Theme; a logo removed from the logo folder since
    /// is none.
    pub(crate) fn look<'a>(&'a self, ballot: &'a Ballot, base: &'a str) -> Look<'a> {
        let (theme, _) = self.resolve(&ballot.theme);
        let logo = ballot
            .logo
            .as_deref()
            .filter(|name| self.logo(name).is_some());
        Look { theme, logo, base }
    }

    /// The logo of this name, if it exists.
    pub(crate) fn logo(&self, name: &str) -> Option<&Logo> {
        self.logos.get(name)
    }

    /// The Theme of this name, if it exists.
    pub(crate) fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.get(name)
    }

    /// The Theme of this name, or the default Theme if there is none (any
    /// more): with the name it goes by.
    pub(crate) fn resolve<'a>(&'a self, name: &'a str) -> (&'a str, &'a Theme) {
        match self.themes.get_key_value(name) {
            Some((name, theme)) => (name, theme),
            None => {
                let (name, theme) = self
                    .themes
                    .get_key_value(DEFAULT_THEME)
                    .expect("the default Theme is built in");
                (name, theme)
            }
        }
    }

    /// Renders `template` of the Theme `name` (the default Theme if it has
    /// gone).
    pub(crate) fn render(
        &self,
        name: &str,
        template: &str,
        context: Value,
    ) -> Result<String, minijinja::Error> {
        let (_, theme) = self.resolve(name);
        theme.templates.get_template(template)?.render(context)
    }
}

/// The subdirectories of the theme folder whose names are valid Theme
/// names, with those names; others are ignored with a warning.
fn theme_dirs(folder: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let entries = std::fs::read_dir(folder)
        .with_context(|| format!("reading the theme folder {}", folder.display()))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("reading the theme folder {}", folder.display()))?;
        let path = entry.path();
        // Follows symlinks, so a Theme may be linked in from elsewhere.
        if !path.is_dir() {
            tracing::warn!(
                "ignoring {} in the theme folder: not a directory",
                path.display()
            );
            continue;
        }
        match entry.file_name().to_str() {
            Some(name) if is_valid_name(name) => dirs.push((name.to_owned(), path)),
            _ => tracing::warn!(
                "ignoring {}: a Theme name consists of lowercase letters, digits and '-' \
                 (starting with a letter or digit, at most 64)",
                path.display()
            ),
        }
    }
    Ok(dirs)
}

/// The folder Theme in `dir`: its `style.css` (required) and the templates
/// in its `templates/` subdirectory that override built-in ones (optional,
/// any of [`templates::TEMPLATE_NAMES`]; other files there are ignored with
/// a warning). Fails, naming the file, if a template doesn't parse or fails
/// the trial render.
fn load_folder_theme(name: &str, dir: &Path) -> anyhow::Result<Theme> {
    let css_path = dir.join("style.css");
    let stylesheet = std::fs::read(&css_path)
        .with_context(|| format!("reading the Theme stylesheet {}", css_path.display()))?;
    let templates_dir = dir.join("templates");
    let mut overrides = Vec::new();
    if templates_dir.is_dir() {
        let entries = std::fs::read_dir(&templates_dir)
            .with_context(|| format!("reading {}", templates_dir.display()))?;
        for entry in entries {
            let path = entry
                .with_context(|| format!("reading {}", templates_dir.display()))?
                .path();
            let known = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| templates::TEMPLATE_NAMES.into_iter().find(|n| *n == name));
            let Some(template) = known else {
                tracing::warn!(
                    "ignoring {}: a Theme's templates are {}",
                    path.display(),
                    templates::TEMPLATE_NAMES.join(", ")
                );
                continue;
            };
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("reading the Theme template {}", path.display()))?;
            overrides.push(templates::Override {
                name: template,
                source,
            });
        }
    }
    let overridden: Vec<&str> = overrides.iter().map(|o| o.name).collect();
    let templates = templates::environment(overrides).map_err(|(name, error)| {
        let path = templates_dir.join(name);
        anyhow::anyhow!(
            "the Theme template {} does not parse: {error:#}",
            path.display()
        )
    })?;
    trial_render::check(name, &templates).map_err(|failure| {
        let base = "base.html";
        let file = match overridden.contains(&failure.template.as_str()) {
            true => templates_dir.join(&failure.template).display().to_string(),
            // The page's content around the form may come from the
            // Theme's `base.html`.
            false if overridden.contains(&base) => format!(
                "built-in {} (extending {})",
                failure.template,
                templates_dir.join(base).display()
            ),
            false => format!("built-in {}", failure.template),
        };
        trial_failure(name, &file, &failure)
    })?;
    Ok(Theme {
        stylesheet: Bytes::from(stylesheet),
        templates,
    })
}

/// The startup error for a Theme failing the trial render: naming the
/// Theme, the template file and the sample, with minijinja's message or what
/// the page lacks.
fn trial_failure(theme: &str, file: &str, failure: &trial_render::Failure) -> anyhow::Error {
    match &failure.problem {
        trial_render::Problem::Render(error) => anyhow::anyhow!(
            "the Theme {theme:?} does not render: template {file} fails on the sample page \
             ({}): {error:#}",
            failure.sample,
        ),
        trial_render::Problem::NoPostForm => anyhow::anyhow!(
            "the Theme {theme:?} has no form to answer with: template {file} renders the \
             sample page ({}) without a <form method=\"post\"> the app can add the hidden \
             revision field to",
            failure.sample,
        ),
        trial_render::Problem::RevisionControl(control) => anyhow::anyhow!(
            "the Theme {theme:?} has a <{control}> named \"revision\": template {file} renders \
             the sample page ({}) with it in a <form method=\"post\">, which the app adds the \
             hidden revision field to; only an <input> may be named \"revision\"",
            failure.sample,
        ),
    }
}

impl ThemeCatalog for Themes {
    fn has_theme(&self, name: &str) -> bool {
        self.themes.contains_key(name)
    }

    fn has_logo(&self, name: &str) -> bool {
        self.logos.contains_key(name)
    }
}
