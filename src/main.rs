#![forbid(unsafe_code)]

use clap::Parser as _;
use vota::cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<std::process::ExitCode> {
    Cli::parse().run().await
}
