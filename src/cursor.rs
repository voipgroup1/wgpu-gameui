//! Cursor state control — let widgets request an OS cursor shape per frame.
//!
//! The library is windowing-agnostic: it never touches winit/SDL directly.
//! Instead, widgets that want a non-default pointer (a text field wants an
//! I-beam, a button wants a hand, a drag handle wants a grab cursor) call
//! [`DrawContext::request_cursor`](crate::DrawContext::request_cursor) during
//! draw. Those requests accumulate into a caller-owned [`CursorState`]; after
//! the frame the application reads [`CursorState::resolve`] (or
//! [`take`](CursorState::take)) and maps the windowing-agnostic [`CursorIcon`]
//! to its own windowing API.
//!
//! ```no_run
//! # use wgpu_gameui::{CursorState, CursorIcon};
//! let mut cursor = CursorState::new();
//! // --- each frame ---
//! cursor.begin_frame();          // clear last frame's request
//! // ... draw the UI; widgets call ctx.request_cursor(..) ...
//! let icon = cursor.resolve();   // CursorIcon::Default if nothing asked
//! // map `icon` to e.g. winit's CursorIcon and call window.set_cursor(..)
//! # let _ = icon;
//! ```

/// A windowing-agnostic cursor shape a widget can request for the current
/// frame. The application maps the resolved icon to its windowing API (e.g.
/// winit's `CursorIcon`); this enum deliberately covers only the shapes UI
/// widgets actually need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorIcon {
    /// The normal arrow pointer.
    #[default]
    Default,
    /// Hand / pointing cursor — over clickable things (buttons, tabs, links).
    Pointer,
    /// I-beam — over editable or selectable text.
    Text,
    /// Open-hand "grab" — over something draggable that isn't being dragged.
    Grab,
    /// Closed-hand "grabbing" — while actively dragging.
    Grabbing,
    /// Horizontal resize (↔) — column dividers, horizontal split handles.
    ResizeHorizontal,
    /// Vertical resize (↕) — row dividers, vertical split handles.
    ResizeVertical,
    /// "Not allowed" — over a disabled or invalid drop target.
    NotAllowed,
}

impl CursorIcon {
    /// Priority used to arbitrate competing requests within a frame: a request
    /// with a higher priority is never clobbered by a lower one, regardless of
    /// draw order. This keeps an active `Grabbing` (set while dragging) winning
    /// over a stray `Pointer`/`Default` from a widget drawn afterwards, even
    /// when the drag has pulled the pointer off the handle.
    fn priority(self) -> u8 {
        match self {
            CursorIcon::Default => 0,
            CursorIcon::Pointer
            | CursorIcon::Text
            | CursorIcon::ResizeHorizontal
            | CursorIcon::ResizeVertical
            | CursorIcon::NotAllowed => 1,
            CursorIcon::Grab => 2,
            // An in-progress drag should beat hover cursors of equal-or-lower rank.
            CursorIcon::Grabbing => 3,
        }
    }
}

/// Caller-owned, per-frame cursor request accumulator.
///
/// Widgets call [`request`](Self::request) (via
/// [`DrawContext::request_cursor`](crate::DrawContext::request_cursor)) while
/// drawing. After the frame the application reads [`resolve`](Self::resolve) and
/// applies the icon to its window. Clear it each frame with
/// [`begin_frame`](Self::begin_frame), or use [`take`](Self::take) to read and
/// clear in one call.
///
/// Conflicts are resolved by `CursorIcon::priority`: the highest-priority
/// request of the frame wins, and among equal priorities the **last** request
/// wins (the topmost widget draws last under back-to-front ordering). In
/// practice only the single topmost hovered widget requests at all, because
/// hover testing already honors [`InputState::mouse_consumed`](crate::InputState::mouse_consumed).
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorState {
    requested: Option<CursorIcon>,
}

impl CursorState {
    /// A fresh accumulator with no request (resolves to [`CursorIcon::Default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Request `icon` for this frame. Kept only if its
    /// `priority` is ≥ the current request's, so a
    /// lower-priority request drawn later can't clobber a higher one.
    pub fn request(&mut self, icon: CursorIcon) {
        match self.requested {
            Some(current) if current.priority() > icon.priority() => {}
            _ => self.requested = Some(icon),
        }
    }

    /// The icon to show this frame — the winning request, or
    /// [`CursorIcon::Default`] if nothing was requested.
    pub fn resolve(&self) -> CursorIcon {
        self.requested.unwrap_or_default()
    }

    /// Whether any widget requested a cursor this frame.
    pub fn is_set(&self) -> bool {
        self.requested.is_some()
    }

    /// Clear the request for the next frame. Call once at the top of each frame
    /// before drawing the UI.
    pub fn begin_frame(&mut self) {
        self.requested = None;
    }

    /// Read the resolved icon and clear in one call (read-and-reset). Handy when
    /// you resolve at the end of the frame and want the accumulator empty for
    /// the next one.
    pub fn take(&mut self) -> CursorIcon {
        let icon = self.resolve();
        self.requested = None;
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_resolves_to_default() {
        let c = CursorState::new();
        assert_eq!(c.resolve(), CursorIcon::Default);
        assert!(!c.is_set());
    }

    #[test]
    fn single_request_wins() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Text);
        assert!(c.is_set());
        assert_eq!(c.resolve(), CursorIcon::Text);
    }

    #[test]
    fn equal_priority_last_wins() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Pointer);
        c.request(CursorIcon::Text); // same priority (1) → last wins
        assert_eq!(c.resolve(), CursorIcon::Text);
    }

    #[test]
    fn higher_priority_not_clobbered_by_lower() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Grabbing); // priority 3
        c.request(CursorIcon::Pointer); // priority 1 — ignored
        c.request(CursorIcon::Default); // priority 0 — ignored
        assert_eq!(c.resolve(), CursorIcon::Grabbing);
    }

    #[test]
    fn higher_priority_overrides_earlier_lower() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Pointer); // 1
        c.request(CursorIcon::Grab); // 2 → wins
        assert_eq!(c.resolve(), CursorIcon::Grab);
    }

    #[test]
    fn begin_frame_and_take_clear_state() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Text);
        c.begin_frame();
        assert_eq!(c.resolve(), CursorIcon::Default);

        c.request(CursorIcon::Pointer);
        assert_eq!(c.take(), CursorIcon::Pointer);
        assert!(!c.is_set(), "take() clears the request");
        assert_eq!(c.resolve(), CursorIcon::Default);
    }

    #[test]
    fn default_icon_is_default() {
        assert_eq!(CursorIcon::default(), CursorIcon::Default);
    }
}
