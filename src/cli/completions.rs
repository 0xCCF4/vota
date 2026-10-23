//! `vota completions <shell>`: a completion script for the whole CLI (the
//! commands of the enabled features), generated from the clap definition.

use std::io::Write as _;

use clap::{CommandFactory as _, ValueEnum};
use clap_complete::Generator;

use super::Cli;

#[derive(clap::Args)]
pub(super) struct CompletionsArgs {
    #[arg(value_enum)]
    shell: Shell,
    /// Complete the NixOS module's `vota-admin-token` wrapper (which runs
    /// `vota admin-token "$@"`) instead of `vota`. For the module's build.
    #[cfg(feature = "server")]
    #[arg(long, hide = true)]
    admin_token_wrapper: bool,
}

/// A shell `completions` writes for.
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
    Nushell,
}

impl CompletionsArgs {
    /// Writes the script to stdout. A reader that stops early (`| head`) is
    /// no error.
    pub(super) fn run(self) -> anyhow::Result<()> {
        #[cfg(feature = "server")]
        let (command, name) = if self.admin_token_wrapper {
            (admin_token_wrapper(), ADMIN_TOKEN_WRAPPER)
        } else {
            (Cli::command(), "vota")
        };
        #[cfg(not(feature = "server"))]
        let (command, name) = (Cli::command(), "vota");
        let script = script(self.shell, command, name);
        match std::io::stdout().lock().write_all(&script) {
            Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => Err(error.into()),
            _ => Ok(()),
        }
    }
}

/// The completion script for `command`, run as `name`.
fn script(shell: Shell, mut command: clap::Command, name: &str) -> Vec<u8> {
    fn generate(generator: impl Generator, command: &mut clap::Command, name: &str) -> Vec<u8> {
        let mut script = Vec::new();
        clap_complete::generate(generator, command, name, &mut script);
        script
    }
    let command = &mut command;
    match shell {
        Shell::Bash => generate(clap_complete::Shell::Bash, command, name),
        Shell::Zsh => generate(clap_complete::Shell::Zsh, command, name),
        Shell::Fish => generate(clap_complete::Shell::Fish, command, name),
        Shell::Elvish => generate(clap_complete::Shell::Elvish, command, name),
        Shell::Powershell => generate(clap_complete::Shell::PowerShell, command, name),
        Shell::Nushell => generate(clap_complete_nushell::Nushell, command, name),
    }
}

/// The command name of the NixOS module's wrapper.
#[cfg(feature = "server")]
const ADMIN_TOKEN_WRAPPER: &str = "vota-admin-token";

/// The `admin-token` subcommand as the NixOS module's `vota-admin-token`
/// wrapper takes it: the same subcommands and flags, without `--config`
/// (the wrapper passes the service's).
#[cfg(feature = "server")]
fn admin_token_wrapper() -> clap::Command {
    Cli::command()
        .find_subcommand("admin-token")
        .expect("a build with `server` has `admin-token`")
        .clone()
        .name(ADMIN_TOKEN_WRAPPER)
        .bin_name(ADMIN_TOKEN_WRAPPER)
}
