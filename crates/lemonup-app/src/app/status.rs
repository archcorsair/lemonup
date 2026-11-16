//! Status messages carry an explicit kind, so how a message is shown (its
//! color, and whether it shows at all) never depends on its wording.

/// What a status message means. The first four are the kinds in the design;
/// `Quiet` records a state change that is not worth showing (for example
/// "selection moved"), so the footer and the toast skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// Something finished well.
    Done,
    /// Something went wrong or is blocked.
    Problem,
    /// Plain information, including a cancelled action or a question.
    Note,
    /// Work has started and is still running.
    Working,
    /// Kept for the record, never shown.
    Quiet,
}

impl StatusKind {
    pub(crate) fn is_shown(self) -> bool {
        !matches!(self, Self::Quiet)
    }
}

/// A key a message offers as the next step. The message stays on the status
/// line for as long as that key still does what the message says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusOffer {
    /// `z` brings back the addons that were just deleted.
    UndoDelete,
    /// `U` updates every addon that has an update ready.
    UpdateAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusMessage {
    pub(crate) kind: StatusKind,
    pub(crate) text: String,
    pub(crate) offer: Option<StatusOffer>,
}

impl StatusMessage {
    pub(crate) fn new(kind: StatusKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            offer: None,
        }
    }

    /// The same message, offering `offer` as the next key.
    pub(crate) fn offering(mut self, offer: StatusOffer) -> Self {
        self.offer = Some(offer);
        self
    }

    /// No message: what closing an overlay leaves behind.
    pub(crate) fn cleared() -> Self {
        Self::new(StatusKind::Quiet, String::new())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl Default for StatusMessage {
    fn default() -> Self {
        Self::cleared()
    }
}
