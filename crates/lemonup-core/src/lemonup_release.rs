//! Whether a newer LemonUp is out: the newest release GitHub named, kept in one small file all
//! games share, and the rules for when to look again and when to say so.
//!
//! The file (`latest-release.json` in the default profile's data folder) holds the newest
//! release's version, when it was published and when LemonUp last looked. It is only an
//! answer to show: a file that is missing or can't be read or parsed means no answer yet, and
//! a failed look never replaces it. A failed look only moves `checked_at`, so the next look
//! waits a day too.

use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::config::write_private_file;
use crate::error::Result;

/// LemonUp looks for a newer release at most this often.
pub const CHECK_EVERY: Duration = Duration::hours(24);
/// A release shows only once it is this old, so the package managers (Homebrew, Scoop, AUR)
/// have it too.
pub const SHOW_AFTER: Duration = Duration::hours(6);

/// A plain `major.minor.patch` version. Anything else (a pre-release suffix, build metadata,
/// missing or extra parts) is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReleaseVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl ReleaseVersion {
    /// The version in a tag such as `v2.8.1` or `2.8.1`, or `None` when it isn't a plain
    /// `major.minor.patch`.
    pub fn parse(tag: &str) -> Option<Self> {
        let text = tag.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut part = || -> Option<u64> {
            let digits = parts.next()?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        };
        let version = Self {
            major: part()?,
            minor: part()?,
            patch: part()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What GitHub said the newest release is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedRelease {
    /// The release's tag, such as `v2.8.1`.
    pub tag: String,
    pub published_at: OffsetDateTime,
}

/// The contents of `latest-release.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestRelease {
    /// The newest release's tag without its leading `v`, such as `2.8.1`. It is kept even when
    /// it isn't a plain version, so LemonUp doesn't look again before the day is up.
    pub version: String,
    #[serde(with = "time::serde::rfc3339")]
    pub published_at: OffsetDateTime,
    /// When LemonUp last looked and got an answer.
    #[serde(with = "time::serde::rfc3339")]
    pub checked_at: OffsetDateTime,
}

impl LatestRelease {
    /// The answer `fetched` gives, looked up at `now`.
    pub fn from_fetched(fetched: &FetchedRelease, now: OffsetDateTime) -> Self {
        let tag = fetched.tag.trim();
        Self {
            version: tag.strip_prefix('v').unwrap_or(tag).to_string(),
            published_at: fetched.published_at,
            checked_at: now,
        }
    }

    /// The newer version to tell about: this release when it is newer than `current` and has
    /// been out for [`SHOW_AFTER`] by `now`.
    pub fn newer_than(&self, current: &str, now: OffsetDateTime) -> Option<ReleaseVersion> {
        let latest = ReleaseVersion::parse(&self.version)?;
        let current = ReleaseVersion::parse(current)?;
        // A date too far out to add to (a damaged file) never shows.
        let shown_from = self.published_at.checked_add(SHOW_AFTER)?;
        (latest > current && now >= shown_from).then_some(latest)
    }
}

/// Whether LemonUp should look for a newer release at `now`: there is no answer yet, the last
/// one is [`CHECK_EVERY`] old, or it says it is from the future (the clock was wrong, or the
/// file is damaged).
pub fn check_due(cached: Option<&LatestRelease>, now: OffsetDateTime) -> bool {
    cached.is_none_or(|cached| {
        now < cached.checked_at
            || cached
                .checked_at
                .checked_add(CHECK_EVERY)
                .is_none_or(|due| now >= due)
    })
}

/// The answer in `path`, or `None` when the file is missing or can't be read or parsed.
pub fn read_latest_release(path: &Path) -> Option<LatestRelease> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Replaces `path` with `release` in one step.
pub fn write_latest_release(path: &Path, release: &LatestRelease) -> Result<()> {
    let mut json = serde_json::to_string_pretty(release)?;
    json.push('\n');
    write_private_file(path, &json)
}

/// Keeps the outcome of a look at `now`: an answer replaces the file in `path` and is returned.
/// A failure keeps the answer in the file and only moves its `checked_at` to `now`, so the next
/// look waits a day too; it is returned as it came. A file that can't be written still gives
/// the answer, for this run.
pub fn record_check<E>(
    path: &Path,
    fetched: std::result::Result<FetchedRelease, E>,
    now: OffsetDateTime,
) -> std::result::Result<LatestRelease, E> {
    let fetched = match fetched {
        Ok(fetched) => fetched,
        Err(error) => {
            if let Some(mut kept) = read_latest_release(path) {
                kept.checked_at = now;
                if let Err(error) = write_latest_release(path, &kept) {
                    tracing::debug!(%error, "couldn't save when LemonUp last looked");
                }
            }
            return Err(error);
        }
    };
    let release = LatestRelease::from_fetched(&fetched, now);
    if let Err(error) = write_latest_release(path, &release) {
        tracing::debug!(%error, "couldn't save the newest LemonUp release");
    }
    Ok(release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(unix: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(unix).expect("time")
    }

    fn release(version: &str, published_at: OffsetDateTime) -> LatestRelease {
        LatestRelease {
            version: version.to_string(),
            published_at,
            checked_at: published_at,
        }
    }

    #[test]
    fn versions_parse_with_or_without_a_v() {
        let expected = ReleaseVersion {
            major: 2,
            minor: 8,
            patch: 1,
        };
        assert_eq!(ReleaseVersion::parse("v2.8.1"), Some(expected));
        assert_eq!(ReleaseVersion::parse("2.8.1"), Some(expected));
        assert_eq!(ReleaseVersion::parse(" v2.8.1\n"), Some(expected));
        assert_eq!(expected.to_string(), "2.8.1");
    }

    #[test]
    fn pre_releases_and_garbage_are_not_versions() {
        for tag in [
            "",
            "v",
            "2.8",
            "2.8.1.4",
            "2.8.1-rc.1",
            "2.8.1+build",
            "v2.8.x",
            "2..1",
            "-2.8.1",
            "+2.8.1",
            "latest",
            "V2.8.1",
        ] {
            assert_eq!(ReleaseVersion::parse(tag), None, "{tag}");
        }
    }

    #[test]
    fn only_a_strictly_newer_plain_version_is_newer() {
        let now = at(1_800_000_000);
        let old = now - Duration::days(2);
        let newer = |version: &str, current: &str| release(version, old).newer_than(current, now);
        assert_eq!(
            newer("2.10.0", "2.9.9").map(|v| v.to_string()),
            Some("2.10.0".into())
        );
        assert_eq!(
            newer("v3.0.0", "2.7.0").map(|v| v.to_string()),
            Some("3.0.0".into())
        );
        assert_eq!(
            newer("2.7.1", "2.7.0").map(|v| v.to_string()),
            Some("2.7.1".into())
        );
        assert_eq!(newer("2.7.0", "2.7.0"), None, "equal is not newer");
        assert_eq!(newer("2.6.9", "2.7.0"), None, "older is not newer");
        assert_eq!(newer("2.9.9", "2.10.0"), None);
        assert_eq!(
            newer("2.8.0-rc.1", "2.7.0"),
            None,
            "a pre-release never shows"
        );
        assert_eq!(newer("nightly", "2.7.0"), None, "garbage never shows");
        assert_eq!(newer("2.8.0", "not-a-version"), None);
    }

    #[test]
    fn a_release_shows_only_after_six_hours() {
        let published = at(1_800_000_000);
        let latest = release("9.9.9", published);
        let shown = |now: OffsetDateTime| latest.newer_than("2.7.0", now).is_some();
        assert!(!shown(published));
        assert!(!shown(
            published + Duration::hours(5) + Duration::minutes(59)
        ));
        assert!(shown(published + Duration::hours(6)));
        assert!(shown(published + Duration::days(3)));
    }

    #[test]
    fn a_check_is_due_with_no_answer_and_after_a_day() {
        let checked = at(1_800_000_000);
        let cached = LatestRelease {
            checked_at: checked,
            ..release("2.8.1", checked - Duration::days(1))
        };
        assert!(check_due(None, checked));
        assert!(!check_due(Some(&cached), checked));
        assert!(!check_due(
            Some(&cached),
            checked + Duration::hours(23) + Duration::minutes(59)
        ));
        assert!(check_due(Some(&cached), checked + Duration::hours(24)));
        assert!(check_due(Some(&cached), checked + Duration::days(5)));
        // A time from the future means the clock was wrong.
        assert!(check_due(Some(&cached), checked - Duration::hours(1)));
    }

    #[test]
    fn the_answer_survives_a_write_and_a_read() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("data").join("latest-release.json");
        let latest = LatestRelease {
            version: "2.8.1".to_string(),
            published_at: at(1_800_000_000),
            checked_at: at(1_800_050_000),
        };
        write_latest_release(&path, &latest).expect("write");
        assert_eq!(read_latest_release(&path), Some(latest));
        let raw = fs::read_to_string(&path).expect("raw");
        assert!(raw.contains("\"version\": \"2.8.1\""), "{raw}");
        assert!(
            raw.contains("\"published_at\": \"2027-01-15T08:00:00Z\""),
            "{raw}"
        );
    }

    #[test]
    fn a_missing_or_unreadable_file_is_no_answer() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("latest-release.json");
        assert_eq!(read_latest_release(&path), None);
        for broken in [
            "",
            "{",
            "null",
            "[]",
            "{\"version\": \"2.8.1\"}",
            "{\"version\": \"2.8.1\", \"published_at\": \"yesterday\", \"checked_at\": \"now\"}",
        ] {
            fs::write(&path, broken).expect("write");
            assert_eq!(read_latest_release(&path), None, "{broken}");
        }
        // A folder where the file should be can't be read either.
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("dir");
        assert_eq!(read_latest_release(&path), None);
    }

    #[test]
    fn dates_too_far_out_never_panic() {
        let far = OffsetDateTime::parse(
            "9999-12-31T23:59:59Z",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("far");
        let damaged = LatestRelease {
            version: "9.9.9".to_string(),
            published_at: far,
            checked_at: far,
        };
        let now = at(1_900_000_000);
        assert_eq!(damaged.newer_than("2.7.0", now), None);
        // From the future: due, so the next look replaces it.
        assert!(check_due(Some(&damaged), now));
        assert!(check_due(Some(&damaged), far));
    }

    #[test]
    fn a_failed_check_keeps_the_old_answer_and_waits_a_day() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("latest-release.json");
        let old = release("2.8.0", at(1_800_000_000));
        write_latest_release(&path, &old).expect("write");

        let failed: std::result::Result<FetchedRelease, &str> = Err("offline");
        let then = at(1_900_000_000);
        assert_eq!(record_check(&path, failed, then), Err("offline"));
        let kept = read_latest_release(&path).expect("kept");
        assert_eq!(kept.version, old.version);
        assert_eq!(kept.published_at, old.published_at);
        assert_eq!(kept.checked_at, then);
        assert!(!check_due(Some(&kept), then + Duration::hours(23)));

        // With no answer yet, a failure writes nothing.
        let missing = temp.path().join("missing.json");
        let failed: std::result::Result<FetchedRelease, &str> = Err("offline");
        assert_eq!(record_check(&missing, failed, then), Err("offline"));
        assert!(!missing.exists());

        let fetched = FetchedRelease {
            tag: "v2.8.1".to_string(),
            published_at: at(1_850_000_000),
        };
        let now = at(1_900_000_000);
        let recorded = record_check::<&str>(&path, Ok(fetched), now).expect("recorded");
        assert_eq!(recorded.version, "2.8.1");
        assert_eq!(recorded.checked_at, now);
        assert_eq!(read_latest_release(&path), Some(recorded));
    }
}
