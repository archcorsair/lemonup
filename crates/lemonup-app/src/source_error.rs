//! What kind of thing went wrong while talking to an addon source (GitHub,
//! Wago.io, TukUI, WoWInterface), so the TUI can say it plainly without
//! matching on error text. The raw error stays in `detail` for bug reports.

use std::fmt;

use reqwest::StatusCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceErrorKind {
    /// The request never got an answer: offline, DNS, refused, timed out.
    NoConnection,
    /// The source refused the key or token LemonUp sent.
    KeyRejected,
    /// The source has no such addon (or repo, or branch).
    NotFound,
    /// The source asked LemonUp to slow down (rate limited).
    SlowDown,
    /// Folders the package installs are already in the AddOns folder and
    /// are not linked: LemonUp may replace them once asked
    /// (`SourceError::clash` names them).
    FoldersInTheWay,
    /// A folder the package installs belongs to an addon LemonUp already
    /// keeps updated: LemonUp won't replace it (`SourceError::clash`).
    FoldersOfLinkedAddon,
    /// A GitHub repository on the known addons list is not the one LemonUp
    /// vetted (GitHub reports another repository id or another owner): it may
    /// have changed owners, so LemonUp won't install, update or check it.
    WrongRepository,
    /// Anything else.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceError {
    pub(crate) kind: SourceErrorKind,
    pub(crate) detail: String,
    /// The folders in the way, for `FoldersInTheWay` and `FoldersOfLinkedAddon`.
    pub(crate) clash: Option<FolderClash>,
}

/// The package folders that are already in the AddOns folder or claimed by
/// a linked addon.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FolderClash {
    /// In the AddOns folder and not linked (no record, or a "Not linked" one).
    pub(crate) replaceable: Vec<String>,
    /// `(folder, addon name)`: the parent or an owned folder of a linked addon.
    pub(crate) linked: Vec<(String, String)>,
    /// Whether an addon in `linked` was linked by a newer LemonUp (it shows as "Not linked"
    /// here, so LemonUp does not keep it updated).
    pub(crate) newer_version_linked: bool,
    /// The name of the "Not linked" addon already there, when one is known.
    pub(crate) existing_name: Option<String>,
    /// The linked addon that is the addon being installed itself, when it is already there and
    /// no other linked addon is in the way. Boxed to keep the error small.
    pub(crate) installed: Option<Box<InstalledAddon>>,
}

/// A linked addon whose own folder is the parent folder of the package being installed:
/// LemonUp knows an addon by that folder, so it is the same addon, already installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledAddon {
    /// The name LemonUp shows for it.
    pub(crate) name: String,
    /// Its own folder: what `lemonup update` selects it by without a doubt.
    pub(crate) folder: String,
    /// Where it is linked to, which is where an update installs it from.
    pub(crate) source: lemonup_core::SourceKind,
}

impl SourceError {
    pub(crate) fn new(kind: SourceErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            clash: None,
        }
    }

    /// A `FoldersInTheWay` or `FoldersOfLinkedAddon` error naming the folders.
    pub(crate) fn folders(
        kind: SourceErrorKind,
        detail: impl Into<String>,
        clash: FolderClash,
    ) -> Self {
        Self {
            kind,
            detail: detail.into(),
            clash: Some(clash),
        }
    }

    /// The kind an HTTP status means, for sources that send a key (Wago.io):
    /// 401 and 403 mean the key was refused.
    pub(crate) fn kind_for_status(status: StatusCode) -> SourceErrorKind {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SourceErrorKind::KeyRejected,
            other => Self::kind_for_keyless_status(other),
        }
    }

    /// The kind an HTTP status means, for sources that send no key (TukUI,
    /// WoWInterface): a 401 or 403 can't be about a key, so it is `Other`.
    pub(crate) fn kind_for_keyless_status(status: StatusCode) -> SourceErrorKind {
        match status {
            StatusCode::NOT_FOUND => SourceErrorKind::NotFound,
            StatusCode::TOO_MANY_REQUESTS => SourceErrorKind::SlowDown,
            _ => SourceErrorKind::Other,
        }
    }

    /// A request error from a source that sends a key: no answer at all is
    /// `NoConnection`, an error status maps through `kind_for_status`.
    pub(crate) fn from_reqwest(error: reqwest::Error) -> Self {
        Self::from_reqwest_with(error, Self::kind_for_status)
    }

    /// `from_reqwest` for sources that send no key (TukUI, WoWInterface).
    pub(crate) fn from_keyless_reqwest(error: reqwest::Error) -> Self {
        Self::from_reqwest_with(error, Self::kind_for_keyless_status)
    }

    fn from_reqwest_with(
        error: reqwest::Error,
        kind_for_status: fn(StatusCode) -> SourceErrorKind,
    ) -> Self {
        let kind = if let Some(status) = error.status() {
            kind_for_status(status)
        } else if error.is_connect() || error.is_timeout() || error.is_request() {
            SourceErrorKind::NoConnection
        } else {
            SourceErrorKind::Other
        };
        Self::new(kind, error.to_string())
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

/// So a command can pass one on with `?` and still read its kind where it prints.
impl std::error::Error for SourceError {}

impl From<String> for SourceError {
    fn from(detail: String) -> Self {
        Self::new(SourceErrorKind::Other, detail)
    }
}

impl From<&str> for SourceError {
    fn from(detail: &str) -> Self {
        Self::new(SourceErrorKind::Other, detail)
    }
}

impl From<SourceError> for String {
    fn from(error: SourceError) -> Self {
        error.detail
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceError, SourceErrorKind};
    use reqwest::StatusCode;

    #[test]
    fn statuses_map_to_kinds() {
        assert_eq!(
            SourceError::kind_for_status(StatusCode::UNAUTHORIZED),
            SourceErrorKind::KeyRejected
        );
        assert_eq!(
            SourceError::kind_for_status(StatusCode::FORBIDDEN),
            SourceErrorKind::KeyRejected
        );
        assert_eq!(
            SourceError::kind_for_status(StatusCode::NOT_FOUND),
            SourceErrorKind::NotFound
        );
        assert_eq!(
            SourceError::kind_for_status(StatusCode::TOO_MANY_REQUESTS),
            SourceErrorKind::SlowDown
        );
        assert_eq!(
            SourceError::kind_for_status(StatusCode::BAD_GATEWAY),
            SourceErrorKind::Other
        );
    }

    #[test]
    fn keyless_sources_never_say_the_key_was_rejected() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert_eq!(
                SourceError::kind_for_keyless_status(status),
                SourceErrorKind::Other
            );
        }
        assert_eq!(
            SourceError::kind_for_keyless_status(StatusCode::NOT_FOUND),
            SourceErrorKind::NotFound
        );
        assert_eq!(
            SourceError::kind_for_keyless_status(StatusCode::TOO_MANY_REQUESTS),
            SourceErrorKind::SlowDown
        );
    }

    #[tokio::test]
    async fn a_refused_connection_is_no_connection() {
        // Port 9 on localhost: nothing listens, so the connect is refused.
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:9/")
            .send()
            .await
            .expect_err("nothing listens on port 9");
        let error = SourceError::from_reqwest(error);
        assert_eq!(error.kind, SourceErrorKind::NoConnection);
        assert!(!error.detail.is_empty());
    }

    #[test]
    fn plain_text_errors_are_other_and_keep_their_text() {
        let error = SourceError::from("zip is broken".to_string());
        assert_eq!(error.kind, SourceErrorKind::Other);
        assert_eq!(String::from(error), "zip is broken");
    }
}
