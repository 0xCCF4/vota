//! The Admin Token files (`admin_token_files`): read once, when the app
//! starts, and kept as hashes only. A missing, unreadable or malformed file
//! stops startup with an error that names the file, never its contents.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use untrusted_value::UntrustedValue;

use crate::domain::admin_token::{FileAdminTokens, ProvisionedAdminToken};
use crate::input;

/// More than an Admin Token file can hold: reading stops one byte after
/// it, so a path such as `/dev/zero` can't stall startup, and a longer file
/// is malformed (its contents are cut short, so they can't pass).
const SIZE_LIMIT: u64 = 4096;

/// The Admin Tokens of these files, in their order.
pub(crate) fn load(paths: &[PathBuf]) -> anyhow::Result<FileAdminTokens> {
    paths
        .iter()
        .map(|path| read(path))
        .collect::<anyhow::Result<_>>()
        .map(FileAdminTokens::new)
}

fn read(path: &Path) -> anyhow::Result<ProvisionedAdminToken> {
    let mut contents = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(SIZE_LIMIT + 1).read_to_end(&mut contents))
        .with_context(|| {
            format!(
                "admin_token_files: cannot read the Admin Token file {}",
                path.display()
            )
        })?;
    if contents.len() as u64 > SIZE_LIMIT {
        // Never a token, whatever its first bytes are.
        contents.clear();
    }
    input::admin_token_file(UntrustedValue::from(contents)).with_context(|| {
        format!(
            "admin_token_files: the Admin Token file {} is malformed",
            path.display()
        )
    })
}
