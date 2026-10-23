//! `--qr` (cargo feature `qr`): a Ballot's URL as a QR code for the
//! terminal, drawn with Unicode half blocks (two modules per character
//! cell) inside the standard quiet zone of four modules.

use qrcode::QrCode;
use qrcode::render::unicode::Dense1x2;
use url::Url;

use super::answer::Show as _;
use super::{Exit, Failure};

/// The QR code of `url` (parsed, so in the `url` crate's ASCII form) and
/// the URL itself below it, for stderr. Light
/// modules (and the quiet zone) are drawn as blocks in the terminal's
/// foreground colour, dark ones are left blank: on the usual light text on
/// a dark background the code shows dark on light, as scanners expect.
pub(super) fn code(url: &Url) -> Result<String, Failure> {
    let code = QrCode::new(url.as_str().as_bytes()).map_err(|error| {
        Failure::new(
            Exit::Other,
            format!("can't draw the URL as a QR code: {error}"),
        )
    })?;
    let drawn = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    Ok(format!("{drawn}\n{}\n", url.show()))
}
