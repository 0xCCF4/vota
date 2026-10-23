//! `vota man <dir>`: one man page per command and subcommand (`vota.1`,
//! `vota-ballot.1`, `vota-ballot-create.1`, …), rendered by `clap_mangen`
//! from the clap definition, plus the sections clap doesn't know:
//! ENVIRONMENT, EXIT STATUS (on the `ballot` pages from
//! `ballot::Exit`), EXAMPLES and SEE ALSO.
//!
//! The pages cover the commands of the enabled features, as `--help` does.

use std::path::Path;

use anyhow::Context as _;
use clap::CommandFactory as _;
use clap_mangen::Man;
use clap_mangen::roff::{Roff, bold, italic, roman};

use super::Cli;

/// The full CLI reference, named on every page.
const REFERENCE: &str = "https://github.com/0xCCF4/vota/blob/main/docs/reference/cli.md";

/// Writes every page into `out_dir`, creating it if missing.
pub(super) fn write_pages(out_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    for (name, page) in pages() {
        let path = out_dir.join(format!("{name}.1"));
        std::fs::write(&path, page).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Every page as `(name, roff source)`, parents before their subcommands.
fn pages() -> Vec<(String, String)> {
    let mut command = Cli::command().disable_help_subcommand(true);
    command.build();
    let mut pages = Vec::new();
    collect(&command, None, &mut pages);
    pages
}

fn collect(command: &clap::Command, parent: Option<&str>, pages: &mut Vec<(String, String)>) {
    let name = page_name(command);
    let children: Vec<&clap::Command> = command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .collect();
    let child_names: Vec<String> = children.iter().map(|sub| page_name(sub)).collect();
    pages.push((name.clone(), render(command, &name, parent, &child_names)));
    for child in children {
        collect(child, Some(&name), pages);
    }
}

/// `vota`, `vota-ballot-create`, …: clap's display name of a built command.
fn page_name(command: &clap::Command) -> String {
    command
        .get_display_name()
        .unwrap_or_else(|| command.get_name())
        .to_owned()
}

fn render(
    command: &clap::Command,
    name: &str,
    parent: Option<&str>,
    children: &[String],
) -> String {
    let man = Man::new(command.clone())
        .title(name.to_uppercase())
        .source(format!("vota {}", env!("CARGO_PKG_VERSION")))
        .manual("Vota Manual");
    let mut page = Vec::new();
    man.render(&mut page)
        .expect("rendering into a Vec never fails");
    let page = String::from_utf8(page).expect("clap_mangen renders UTF-8");

    let mut extra = Roff::new();
    environment(&mut extra, name);
    exit_status(&mut extra, name);
    examples(&mut extra, name);
    notes(&mut extra, name);
    see_also(&mut extra, parent, children);
    // Without the preamble every rendered Roff starts with.
    let preamble = Roff::new().render();
    let extra = extra.render();
    let extra = &extra[preamble.len()..];

    // Before VERSION, which clap_mangen puts last.
    match page.find("\n.SH VERSION\n") {
        Some(at) => format!("{}\n{extra}{}", &page[..at], &page[at + 1..]),
        None => format!("{page}{extra}"),
    }
}

fn is_ballot(name: &str) -> bool {
    name == "vota-ballot" || name.starts_with("vota-ballot-")
}

/// The commands that load the configuration and log.
fn is_server(name: &str) -> bool {
    name == "vota-serve"
        || (name.starts_with("vota-admin-token") && name != "vota-admin-token-generate")
}

fn section(roff: &mut Roff, title: &str) {
    roff.control("SH", [title]);
}

/// A tagged paragraph: `tag` on its own line, `text` indented below.
fn item(roff: &mut Roff, tag: &str, text: &str) {
    roff.control("TP", []).text([bold(tag)]).text([roman(text)]);
}

/// Lines shown as they are, e.g. commands.
fn literal(roff: &mut Roff, lines: &[&str]) {
    roff.control("PP", [])
        .control("nf", [])
        .control("RS", ["4"]);
    for line in lines {
        roff.text([roman(*line)]);
    }
    roff.control("RE", []).control("fi", []);
}

fn environment(roff: &mut Roff, name: &str) {
    let top = name == "vota";
    let server = (top && cfg!(feature = "server")) || is_server(name);
    let client = (top && cfg!(feature = "client")) || is_ballot(name);
    if !server && !client {
        return;
    }
    section(roff, "ENVIRONMENT");
    if server {
        item(roff, "VOTA_CONFIG", "The TOML config file, as --config.");
        item(
            roff,
            "VOTA_<OPTION>",
            "Overrides a configuration option of the file: VOTA_ and the option's name in \
             upper case, with a double underscore into a table, e.g. VOTA_PORT=9000 or \
             VOTA_RATE_LIMIT__WINDOW=30m. An unknown VOTA_* variable is ignored with a warning.",
        );
        item(
            roff,
            "RUST_LOG",
            "The log level of the server commands (default info), e.g. debug. Logs go to stderr.",
        );
    }
    if client {
        item(
            roff,
            "VOTA_URL",
            "The server's public base URL, sub-path included, as --url (ballot).",
        );
        item(
            roff,
            "VOTA_ADMIN_TOKEN",
            "The Admin Token, used when neither --token nor --token-file is given (ballot). \
             Prefer it, or --token-file, to --token, which other users see in ps.",
        );
    }
}

fn exit_status(roff: &mut Roff, name: &str) {
    section(roff, "EXIT STATUS");
    #[cfg(feature = "client")]
    if is_ballot(name) {
        for exit in super::ballot::Exit::ALL {
            item(roff, &exit.code().to_string(), exit.description());
        }
        return;
    }
    item(roff, "0", "Success.");
    item(roff, "1", "Runtime error, printed to stderr after Error:.");
    item(
        roff,
        "2",
        "Usage error from argument parsing (unknown subcommand, invalid or conflicting flags).",
    );
    if name == "vota" && cfg!(feature = "client") {
        roff.control("PP", []).text([
            roman("vota ballot has exit codes of its own, see "),
            bold("vota-ballot"),
            roman("(1)."),
        ]);
    }
}

fn examples(roff: &mut Roff, name: &str) {
    let lines: &[&str] = match name {
        "vota-serve" => &["vota --config /etc/vota/vota.toml serve"],
        "vota-admin-token-create" => {
            &["vota --config vota.toml admin-token create --label ci --expires-in 90days"]
        }
        "vota-admin-token-generate" => &["(umask 077; vota admin-token generate > admin-token)"],
        "vota-ballot" | "vota-ballot-create" => &[
            "export VOTA_URL=https://example.org/polls/",
            "export VOTA_ADMIN_TOKEN=\"$(cat ~/.config/vota/token)\"",
            "vota ballot create --title Lunch --option Pizza --option Sushi --voting-deadline 3d",
            "vota ballot create --title Survey --option Yes --option No --voting-deadline 14d \
             --group staff --voters voters.txt --csv > links.csv",
            "vota ballot list --group lunch",
            "vota ballot tally --group lunch",
            "vota ballot list --group lunch --csv > lunch.csv",
            "vota ballot extend 1 --voting-deadline +1d",
        ],
        "vota-completions" => &[
            "vota completions bash > ~/.local/share/bash-completion/completions/vota",
            "vota completions zsh > ~/.zfunc/_vota",
            "vota completions fish > ~/.config/fish/completions/vota.fish",
        ],
        "vota-man" => &["vota man ~/.local/share/man/man1", "man vota-ballot-create"],
        _ => return,
    };
    section(roff, "EXAMPLES");
    literal(roff, lines);
}

fn notes(roff: &mut Roff, name: &str) {
    if name != "vota-admin-token" {
        return;
    }
    section(roff, "NOTES");
    roff.text([
        roman("On NixOS, the module services.vota installs a wrapper "),
        bold("vota-admin-token"),
        roman(
            ", run as root, that runs vota admin-token with the same subcommands and flags \
             as the service's user, against the service's configuration and database, e.g. ",
        ),
        italic("sudo vota-admin-token create --label ci"),
        roman("."),
    ]);
}

fn see_also(roff: &mut Roff, parent: Option<&str>, children: &[String]) {
    section(roff, "SEE ALSO");
    let mut inlines = Vec::new();
    for page in parent
        .into_iter()
        .chain(children.iter().map(String::as_str))
    {
        inlines.push(bold(page));
        inlines.push(roman("(1), "));
    }
    inlines.push(roman("and the CLI reference: "));
    inlines.push(roman(REFERENCE));
    roff.text(inlines);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_the_extra_sections_before_version() {
        for (name, page) in pages() {
            let see_also = page
                .find(".SH \"SEE ALSO\"")
                .or_else(|| page.find(".SH SEE ALSO"));
            let see_also = see_also.unwrap_or_else(|| panic!("{name}: no SEE ALSO\n{page}"));
            assert!(
                page.contains(".SH \"EXIT STATUS\"") || page.contains(".SH EXIT STATUS"),
                "{name}"
            );
            if let Some(version) = page.find(".SH VERSION") {
                assert!(see_also < version, "{name}");
            }
            // One preamble only, at the top.
            assert_eq!(page.matches(".ds Aq").count(), 2, "{name}");
        }
    }
}
