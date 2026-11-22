//! A newer LemonUp: the answer kept from the last look on GitHub, the one look a launch takes,
//! and the header badge that says so.

use lemonup_core::lemonup_release::{
    LatestRelease, ReleaseVersion, check_due, read_latest_release,
};

use super::*;
use crate::shimmer::working_text_spans;

/// The newest LemonUp release the app knows of. It is kept when the app switches games, so a
/// launch looks only once.
pub(super) struct NewerLemonupState {
    /// The answer from `latest-release.json`, or from this launch's look.
    pub(super) latest: Option<LatestRelease>,
    /// This launch's look started (or wasn't due); it never starts again.
    pub(super) started: bool,
    pub(super) sender: mpsc::UnboundedSender<LatestRelease>,
    receiver: mpsc::UnboundedReceiver<LatestRelease>,
    /// The last frame drew the badge, so its shimmer needs redraws.
    pub(super) badge_drawn: std::cell::Cell<bool>,
}

impl NewerLemonupState {
    /// The kept answer of every game, from `roots`.
    pub(super) fn read(roots: &lemonup_core::ProfileRoots) -> Self {
        Self::with(read_latest_release(&roots.latest_release_file()))
    }

    pub(super) fn with(latest: Option<LatestRelease>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            latest,
            started: false,
            sender,
            receiver,
            badge_drawn: std::cell::Cell::new(false),
        }
    }
}

impl App {
    /// The newer LemonUp to tell about at `now`: the setting is on, and the release is newer
    /// than this one and old enough.
    pub(super) fn newer_lemonup(&self, now: OffsetDateTime) -> Option<ReleaseVersion> {
        if !self.config_pane.draft.check_lemonup_updates {
            return None;
        }
        self.newer_lemonup
            .latest
            .as_ref()?
            .newer_than(env!("CARGO_PKG_VERSION"), now)
    }

    /// Starts this launch's look on GitHub in the background once the addon list shows, when
    /// the setting is on and the last answer is a day old. A launch looks at most once; with the
    /// setting off it waits, and turning it on looks then.
    pub(super) fn start_newer_lemonup_check(&mut self, now: OffsetDateTime) {
        let state = &mut self.newer_lemonup;
        if state.started
            || self.shell_mode != ShellMode::Dashboard
            || !self.config_pane.draft.check_lemonup_updates
        {
            return;
        }
        state.started = true;
        if !check_due(state.latest.as_ref(), now) {
            return;
        }
        let cache = self.roots.latest_release_file();
        let sender = state.sender.clone();
        tokio::spawn(async move {
            match crate::lemonup_release::check_latest_release(&cache).await {
                Ok(latest) => {
                    let _ = sender.send(latest);
                }
                // Silent: the answer stays, and the next look waits a day.
                Err(error) => {
                    tracing::debug!(kind = ?error.kind, "couldn't look for a newer LemonUp")
                }
            }
        });
    }

    /// Takes the answer of this launch's look, when it came.
    pub(super) fn take_newer_lemonup_answer(&mut self) {
        while let Ok(latest) = self.newer_lemonup.receiver.try_recv() {
            self.newer_lemonup.latest = Some(latest);
        }
    }

    /// Hands the look and its answer to the app of the game switched to.
    pub(super) fn keep_newer_lemonup(&mut self, next: &mut App) {
        std::mem::swap(&mut next.newer_lemonup, &mut self.newer_lemonup);
    }

    /// The header's `↑ Update available 2.7.0 → 2.8.1`, the words shimmering unless motion is
    /// reduced, or nothing.
    pub(super) fn newer_lemonup_badge(&self, now: OffsetDateTime) -> Vec<Span<'static>> {
        let Some(version) = self.newer_lemonup(now) else {
            return Vec::new();
        };
        let roles = self.ui_theme.roles;
        let mut spans = vec![
            Span::styled(
                "↑",
                Style::default()
                    .fg(roles.amber)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ];
        spans.extend(working_text_spans(
            &format!(
                "Update available {} {} {version}",
                env!("CARGO_PKG_VERSION"),
                self.ui_theme.symbols.arrow
            ),
            roles.amber,
            roles.lemon,
            self.ui_theme.reduce_motion,
        ));
        spans
    }

    /// The last frame drew a shimmering badge, so the frame keeps redrawing.
    pub(super) fn newer_lemonup_shimmers(&self) -> bool {
        self.newer_lemonup.badge_drawn.get()
            && !self.ui_theme.reduce_motion
            && matches!(self.ui_theme.roles.amber, Color::Rgb(..))
    }
}
