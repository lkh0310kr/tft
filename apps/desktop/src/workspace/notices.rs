use gpui::{Context, SharedString};
use std::time::Duration;

use super::Workspace;

/// How long a notice stays on screen; the observed duration is approximate.
pub(super) const NOTICE_LIFETIME: Duration = Duration::from_secs(5);

/// A transient message and the window in which it should appear.
pub(super) struct Notice {
    pub(super) id: u64,
    pub(super) text: SharedString,
    /// The window it shows in (`None` for the vault window), active when it appeared.
    pub(super) window: Option<u64>,
}

impl Notice {
    /// The window it draws in: its own while open, else the vault window.
    fn home(&self, open: &impl Fn(u64) -> bool) -> Option<u64> {
        self.window.filter(|id| open(*id))
    }
}

/// Transient messages, oldest first. Command outcomes expire independently of status text.
#[derive(Default)]
pub(super) struct Notices {
    live: Vec<Notice>,
    last_id: u64,
}

/// Stable status and expiring notices share the workspace's user-feedback surface.
pub(super) struct FeedbackState {
    status: SharedString,
    pub(super) notices: Notices,
}

impl FeedbackState {
    pub(super) fn new(status: impl Into<SharedString>) -> Self {
        Self {
            status: status.into(),
            notices: Notices::default(),
        }
    }

    pub(super) fn status_text(&self) -> String {
        self.notices
            .latest()
            .map_or_else(|| self.status.to_string(), ToString::to_string)
    }

    pub(super) fn set_status(&mut self, status: impl Into<SharedString>) {
        self.status = status.into();
    }

    pub(super) fn clear_status_text(&mut self) {
        self.status = SharedString::default();
    }

    pub(super) fn clear(&mut self) {
        self.notices.clear();
        self.status = "".into();
    }
}

impl Workspace {
    /// Empties the status bar and takes down every notice, so a caller
    /// can tell the next notice from the one already there.
    pub fn clear_status(&mut self, cx: &mut Context<Self>) {
        self.feedback.clear();
        cx.notify();
    }
}

impl Notices {
    /// Shows `text` in `window` (`None` for the vault) and returns its id.
    /// A repeated message is renewed rather than stacked.
    pub(super) fn push(&mut self, text: SharedString, window: Option<u64>) -> u64 {
        self.live.retain(|notice| notice.text != text);
        self.last_id += 1;
        self.live.push(Notice {
            id: self.last_id,
            text,
            window,
        });
        self.last_id
    }

    /// Notices rendered by `here`, oldest first.
    pub(super) fn in_window<'a>(
        &'a self,
        here: Option<u64>,
        open: &'a impl Fn(u64) -> bool,
    ) -> impl Iterator<Item = &'a Notice> + 'a {
        self.live
            .iter()
            .filter(move |notice| notice.home(open) == here)
    }

    /// The window each message draws in, in the order of `texts`.
    pub(super) fn windows(&self, open: &impl Fn(u64) -> bool) -> Vec<Option<u64>> {
        self.live.iter().map(|notice| notice.home(open)).collect()
    }

    /// Drops the message with this id, if it is still on screen.
    pub(super) fn expire(&mut self, id: u64) {
        self.live.retain(|notice| notice.id != id);
    }

    pub(super) fn clear(&mut self) {
        self.live.clear();
    }

    /// The newest message.
    pub(super) fn latest(&self) -> Option<&SharedString> {
        self.live.last().map(|notice| &notice.text)
    }

    pub(super) fn texts(&self) -> Vec<String> {
        self.live
            .iter()
            .map(|notice| notice.text.to_string())
            .collect()
    }
}
