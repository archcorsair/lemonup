//! How many requests LemonUp sends at once, in total and to each addon source, so checking
//! many addons stays polite to the sources. Each check run makes its own limits, so the caps
//! hold for one run: a check started in front never waits behind a background check of
//! another game.

use std::sync::Arc;

use lemonup_core::SourceKind;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Requests out at once, across every source.
const TOTAL: usize = 6;
const GITHUB: usize = 3;
const WAGO: usize = 4;
const WOWINTERFACE: usize = 4;
const TUKUI: usize = 2;

#[derive(Debug, Clone)]
pub(crate) struct SourceLimits {
    total: Arc<Semaphore>,
    github: Arc<Semaphore>,
    wago: Arc<Semaphore>,
    wowinterface: Arc<Semaphore>,
    tukui: Arc<Semaphore>,
}

/// Leave to send one request: the source's permit and one of the total permits. Dropping it
/// gives both back.
#[derive(Debug)]
pub(crate) struct SourcePermit {
    _source: Option<OwnedSemaphorePermit>,
    _total: Option<OwnedSemaphorePermit>,
}

impl SourceLimits {
    pub(crate) fn new(
        total: usize,
        github: usize,
        wago: usize,
        wowinterface: usize,
        tukui: usize,
    ) -> Self {
        Self {
            total: Arc::new(Semaphore::new(total)),
            github: Arc::new(Semaphore::new(github)),
            wago: Arc::new(Semaphore::new(wago)),
            wowinterface: Arc::new(Semaphore::new(wowinterface)),
            tukui: Arc::new(Semaphore::new(tukui)),
        }
    }

    /// Fresh limits with the fixed caps, for one check run. The caps hold for that run only.
    pub(crate) fn standard() -> Self {
        Self::new(TOTAL, GITHUB, WAGO, WOWINTERFACE, TUKUI)
    }

    /// Waits for leave to send one request to `source`: the source's own permit first, then
    /// a total one, so a busy source never holds total permits another source could use.
    /// Manual addons have no source to ask and get an empty permit right away.
    pub(crate) async fn acquire(&self, source: SourceKind) -> SourcePermit {
        let semaphore = match source {
            SourceKind::Manual => {
                return SourcePermit {
                    _source: None,
                    _total: None,
                };
            }
            SourceKind::GitHub => &self.github,
            SourceKind::Wago => &self.wago,
            SourceKind::WowInterface => &self.wowinterface,
            SourceKind::Tukui => &self.tukui,
        };
        // The semaphores are never closed, so acquiring only fails in theory.
        let source_permit = Arc::clone(semaphore).acquire_owned().await.ok();
        let total_permit = Arc::clone(&self.total).acquire_owned().await.ok();
        SourcePermit {
            _source: source_permit,
            _total: total_permit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_permit_takes_one_of_the_source_and_one_of_the_total() {
        let limits = SourceLimits::new(2, 1, 1, 1, 1);
        let permit = limits.acquire(SourceKind::Wago).await;
        assert_eq!(limits.wago.available_permits(), 0);
        assert_eq!(limits.total.available_permits(), 1);
        assert_eq!(limits.github.available_permits(), 1);
        drop(permit);
        assert_eq!(limits.wago.available_permits(), 1);
        assert_eq!(limits.total.available_permits(), 2);

        let _manual = limits.acquire(SourceKind::Manual).await;
        assert_eq!(limits.total.available_permits(), 2);
    }

    #[test]
    fn the_standard_limits_have_the_fixed_caps() {
        let limits = SourceLimits::standard();
        assert_eq!(limits.total.available_permits(), TOTAL);
        assert_eq!(limits.github.available_permits(), GITHUB);
        assert_eq!(limits.wago.available_permits(), WAGO);
        assert_eq!(limits.wowinterface.available_permits(), WOWINTERFACE);
        assert_eq!(limits.tukui.available_permits(), TUKUI);
    }
}
