//! Drag-capture arbitration: a single owner for an in-progress pointer drag.
//!
//! Immediate-mode draggable widgets (sliders, scroll thumbs, window movers, ...)
//! each receive a stable [`DragId`] and a shared `&mut DragCapture`. On
//! mouse-down a widget *requests* capture; the request only succeeds if no
//! other widget currently owns the drag. While the gesture is held, the owner
//! is the only widget that reacts, so overlapping or adjacent draggables can't
//! both follow a single mouse. Releasing the mouse frees the capture.
//!
//! `DragCapture` is caller-owned and persists across frames — construct one per
//! UI surface and thread `&mut` into every draggable you draw, the same way the
//! crate already threads caller-owned `ScrollState` into `ScrollView`.

/// Stable identity for a draggable widget within one UI surface.
///
/// Any scheme that is unique per draggable per frame works: a hash of a widget
/// path, an enum discriminant, a loop index, etc. `0` is a valid id.
pub type DragId = u64;

/// Arbitrates which draggable widget currently owns the pointer drag.
///
/// At most one [`DragId`] can hold the capture at a time. Caller-owned;
/// persists across frames.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DragCapture {
    active: Option<DragId>,
}

impl DragCapture {
    /// A fresh capture with no active drag.
    pub fn new() -> Self {
        Self::default()
    }

    /// The widget that currently owns the drag, if any.
    pub fn active(&self) -> Option<DragId> {
        self.active
    }

    /// True when `id` currently owns the drag.
    pub fn is_active(&self, id: DragId) -> bool {
        self.active == Some(id)
    }

    /// True when no widget owns the drag and a new one may begin.
    pub fn is_free(&self) -> bool {
        self.active.is_none()
    }

    /// Request the drag for `id`. Succeeds (returns `true`) only when the
    /// capture is free or already held by `id`; idempotent for the current
    /// owner. Returns `false` when a *different* widget already owns the drag.
    pub fn try_begin(&mut self, id: DragId) -> bool {
        match self.active {
            None => {
                self.active = Some(id);
                true
            }
            Some(cur) => cur == id,
        }
    }

    /// Release the drag if `id` owns it. No-op when a different widget owns it
    /// or nothing is active, so every draggable can safely call this each frame
    /// on mouse-up without stealing or clobbering another's capture.
    pub fn release(&mut self, id: DragId) {
        if self.active == Some(id) {
            self.active = None;
        }
    }

    /// Force-clear any active drag, regardless of owner (e.g. on focus loss,
    /// window blur, or a cancel key).
    pub fn clear(&mut self) {
        self.active = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_capture_is_free() {
        let cap = DragCapture::new();
        assert!(cap.is_free());
        assert_eq!(cap.active(), None);
        assert!(!cap.is_active(0));
    }

    #[test]
    fn try_begin_claims_when_free() {
        let mut cap = DragCapture::new();
        assert!(cap.try_begin(7));
        assert!(!cap.is_free());
        assert!(cap.is_active(7));
        assert_eq!(cap.active(), Some(7));
    }

    #[test]
    fn try_begin_is_idempotent_for_owner() {
        let mut cap = DragCapture::new();
        assert!(cap.try_begin(7));
        // Re-requesting by the same owner keeps the capture and succeeds.
        assert!(cap.try_begin(7));
        assert!(cap.is_active(7));
    }

    #[test]
    fn try_begin_rejects_other_while_held() {
        let mut cap = DragCapture::new();
        assert!(cap.try_begin(1));
        // A different widget cannot steal an active drag.
        assert!(!cap.try_begin(2));
        assert!(cap.is_active(1));
        assert!(!cap.is_active(2));
    }

    #[test]
    fn release_only_frees_owner() {
        let mut cap = DragCapture::new();
        cap.try_begin(1);
        // A non-owner releasing is a no-op.
        cap.release(2);
        assert!(cap.is_active(1));
        // The owner releasing frees the capture.
        cap.release(1);
        assert!(cap.is_free());
    }

    #[test]
    fn clear_drops_any_owner() {
        let mut cap = DragCapture::new();
        cap.try_begin(42);
        cap.clear();
        assert!(cap.is_free());
    }
}
