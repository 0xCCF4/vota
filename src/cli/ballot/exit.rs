//! The exit codes of `vota ballot`, the one place they are defined: the
//! number, which HTTP statuses lead to it, and the text of the man pages'
//! EXIT STATUS. The CLI reference documents the same table by hand;
//! `docs_drift` (a test) compares the two.

use std::process::ExitCode;

use reqwest::StatusCode;

/// How `vota ballot` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Success = 0,
    /// Anything without a code of its own.
    Other = 1,
    /// Refused before anything was sent, like clap's own usage errors.
    Usage = 2,
    NotFound = 3,
    Conflict = 4,
    /// The server refused the request's content.
    Refused = 5,
    /// The Admin Token was refused, or the client is locked out.
    Denied = 6,
    /// No answer arrived.
    Network = 7,
}

impl Exit {
    /// Every exit code, by number.
    pub(crate) const ALL: [Self; 8] = [
        Self::Success,
        Self::Other,
        Self::Usage,
        Self::NotFound,
        Self::Conflict,
        Self::Refused,
        Self::Denied,
        Self::Network,
    ];

    pub(crate) fn code(self) -> u8 {
        self as u8
    }

    /// When the code is returned, for the man pages; the CLI reference has
    /// the same text, in Markdown.
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Success => "Success.",
            Self::Other => {
                "Anything else: a 5xx, a redirect, an unexpected answer, or a declined delete \
                 prompt."
            }
            Self::Usage => {
                "Usage error: from clap (unknown or conflicting flags such as --csv with \
                 --json, a bad --choice-mode, --submission-mode or --status value), no Admin \
                 Token, no URL, a refused URL, \
                 an unreadable --token-file, --description-file, --body, --voters or \
                 --ca-cert, a --body that is not JSON, --voter-reference more than once on \
                 edit, --qr when creating many Ballots, a date flag that is neither a \
                 timestamp nor a relative date (or a + with nothing to be relative to), or \
                 delete without a terminal and without --yes. Nothing was sent."
            }
            Self::NotFound => "404: no live Ballot with this ID (or a wrong --url).",
            Self::Conflict => {
                "409: the Ballot can't be edited now, or a Voter Reference is taken in the \
                 Group (or --count can't number on)."
            }
            Self::Refused => "400, 413, 415 or 422: the server refused the request's content.",
            Self::Denied => {
                "401 or 429: the Admin Token was refused, or this client's address is locked \
                 out for a while."
            }
            Self::Network => "Network error, TLS error or timeout: no answer arrived.",
        }
    }

    /// The HTTP error statuses that end with this code; any other error
    /// status ends with [`Self::Other`].
    pub(crate) fn statuses(self) -> &'static [u16] {
        match self {
            Self::NotFound => &[404],
            Self::Conflict => &[409],
            Self::Refused => &[400, 413, 415, 422],
            Self::Denied => &[401, 429],
            Self::Success | Self::Other | Self::Usage | Self::Network => &[],
        }
    }

    /// The code for an error answer with `status`.
    pub(super) fn for_status(status: StatusCode) -> Self {
        Self::ALL
            .into_iter()
            .find(|exit| exit.statuses().contains(&status.as_u16()))
            .unwrap_or(Self::Other)
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_are_numbered_as_they_always_were() {
        let codes: Vec<u8> = Exit::ALL.iter().map(|exit| exit.code()).collect();

        assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(Exit::Usage.code(), 2);
        assert_eq!(Exit::Network.code(), 7);
    }

    #[test]
    fn each_error_status_has_the_code_it_always_had() {
        for (status, code) in [
            (400, 5),
            (401, 6),
            (403, 1),
            (404, 3),
            (405, 1),
            (409, 4),
            (413, 5),
            (415, 5),
            (422, 5),
            (429, 6),
            (500, 1),
            (502, 1),
            (503, 1),
        ] {
            let status = StatusCode::from_u16(status).unwrap();

            assert_eq!(Exit::for_status(status).code(), code, "{status}");
        }
    }
}
