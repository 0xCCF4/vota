//! Logos for the logo Theme Tweak: `<logo_folder>/<name>.{svg,png,webp}`,
//! loaded once at startup (a restart picks up changes).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use axum::body::Bytes;

use crate::domain::theme::is_valid_name;

/// Largest logo file accepted.
pub(crate) const MAX_LOGO_BYTES: u64 = 512 * 1024;

/// The kinds of logo file, by extension (exactly, lowercase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogoKind {
    Svg,
    Png,
    Webp,
}

impl LogoKind {
    const ALL: [Self; 3] = [Self::Svg, Self::Png, Self::Webp];

    fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Self::Svg => "image/svg+xml",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }

    /// Whether `bytes` look like a file of this kind: the PNG signature, a
    /// RIFF container of type WEBP, or UTF-8 text with an `<svg` element.
    fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Webp => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
            Self::Svg => std::str::from_utf8(bytes).is_ok_and(|text| text.contains("<svg")),
        }
    }
}

/// One logo file's contents.
pub(crate) struct Logo {
    pub(crate) kind: LogoKind,
    pub(crate) bytes: Bytes,
}

/// The logos in `folder`, by name. Files that aren't `<name>.svg`, `.png` or
/// `.webp` with a valid name are ignored with a warning. Fails, naming the
/// file(s), if two files share a name, a logo exceeds [`MAX_LOGO_BYTES`], or
/// its contents don't match its extension.
pub(crate) fn load(folder: &Path) -> anyhow::Result<BTreeMap<String, Logo>> {
    let context = || format!("reading the logo folder {}", folder.display());
    let mut logos: BTreeMap<String, (PathBuf, Logo)> = BTreeMap::new();
    for entry in std::fs::read_dir(folder).with_context(context)? {
        let path = entry.with_context(context)?.path();
        let Some((name, kind)) = logo_file_name(&path) else {
            tracing::warn!(
                "ignoring {} in the logo folder: a logo is a file <name>.svg, <name>.png or \
                 <name>.webp, its name of lowercase letters, digits and '-' (starting with a \
                 letter or digit, at most 64)",
                path.display()
            );
            continue;
        };
        // Follows symlinks, so a logo may be linked in from elsewhere.
        let metadata =
            std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        if !metadata.is_file() {
            tracing::warn!("ignoring {} in the logo folder: not a file", path.display());
            continue;
        }
        if let Some((other, _)) = logos.get(name) {
            bail!(
                "two logo files for the name {name:?}: {} and {}; keep one",
                other.display(),
                path.display()
            );
        }
        if metadata.len() > MAX_LOGO_BYTES {
            bail!(
                "the logo {} is larger than {} KiB",
                path.display(),
                MAX_LOGO_BYTES / 1024
            );
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() as u64 > MAX_LOGO_BYTES {
            bail!(
                "the logo {} is larger than {} KiB",
                path.display(),
                MAX_LOGO_BYTES / 1024
            );
        }
        if !kind.matches(&bytes) {
            bail!(
                "the logo {} is not a valid {} file",
                path.display(),
                kind.extension()
            );
        }
        let logo = Logo {
            kind,
            bytes: Bytes::from(bytes),
        };
        logos.insert(name.to_owned(), (path, logo));
    }
    Ok(logos
        .into_iter()
        .map(|(name, (_, logo))| (name, logo))
        .collect())
}

/// `<name>.<extension>` with a valid name and a known extension.
fn logo_file_name(path: &Path) -> Option<(&str, LogoKind)> {
    let file_name = path.file_name()?.to_str()?;
    let (name, extension) = file_name.rsplit_once('.')?;
    let kind = LogoKind::ALL
        .into_iter()
        .find(|kind| kind.extension() == extension)?;
    is_valid_name(name).then_some((name, kind))
}
