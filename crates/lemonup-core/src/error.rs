use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LemonupError {
    #[error("unable to resolve LemonUp config/data directories for this platform")]
    PathsUnavailable,
    #[error("missing config file at {0}")]
    ConfigMissing(PathBuf),
    #[error(
        "couldn't read {}: {reason} | nothing was changed | fix or move that file, then run LemonUp again",
        path.display()
    )]
    SettingsUnreadable { path: PathBuf, reason: String },
    /// Another game needed the default game's settings, for the settings all games share.
    #[error(
        "couldn't read the settings of your default game at {}: {reason} | nothing was changed | fix that file, then run LemonUp again",
        path.display()
    )]
    DefaultSettingsUnreadable { path: PathBuf, reason: String },
    /// A change to the settings all games share, while another game's config can't be read:
    /// `shared.toml` can't be made before then, and the change would be lost.
    #[error(
        "this change waits until {} can be read: every game shares it | nothing was changed | fix or move that file, then try again",
        list_paths(paths)
    )]
    SharedChangeWaits { paths: Vec<PathBuf> },
    #[error(
        "{} was saved by a newer LemonUp | nothing was changed | update LemonUp to change these settings",
        path.display()
    )]
    SettingsTooNew { path: PathBuf },
    #[error("another LemonUp is saving settings | nothing was changed | try again")]
    SharedSettingsBusy,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A command can't do what it was asked the way it was asked. `reason` says what is
    /// wrong and can hold names or paths that were typed or read. `hint` is LemonUp's own
    /// advice on what to run or pass instead, kept apart so that only it is shown as naming
    /// commands. Either can be empty.
    #[error("invalid argument: {}", join_sentences(reason, hint))]
    Usage { reason: String, hint: String },
    /// A command declined to act and changed nothing. The message says what happened and what
    /// to do next, so it is shown as it is, without an error label.
    #[error("{0}")]
    Refused(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("toml deserialize error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// `first` and `second` as one line: a space after a finished sentence, else the ` | ` that
/// LemonUp's longer messages use between their parts.
fn join_sentences(first: &str, second: &str) -> String {
    if first.is_empty() || second.is_empty() {
        return format!("{first}{second}");
    }
    let separator = if first.ends_with(['.', '!', '?']) {
        " "
    } else {
        " | "
    };
    format!("{first}{separator}{second}")
}

fn list_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" and ")
}

pub type Result<T> = std::result::Result<T, LemonupError>;
