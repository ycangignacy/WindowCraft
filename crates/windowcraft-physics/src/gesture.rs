// Made by Nordan
//! Pointer gesture state machines.
//!
//! The UI layer feeds raw pointer samples in and reads back decisions, which
//! keeps hysteresis, cancellation and commit logic testable without a window.

use crate::velocity::VelocityTracker;

/// Movement required before a drag locks to an axis.
pub const DRAG_THRESHOLD: f64 = 10.0;
/// Extra hit area around a pressed target.
pub const HIT_PADDING: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Idle,
    /// Pointer is down but has not crossed the threshold.
    Pending,
    /// Locked to the configured axis and tracking 1:1.
    Dragging,
    /// Moved past the threshold on the other axis, so a scroll or similar
    /// should own this pointer instead.
    Rejected,
}

/// What a released drag should do next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Release {
    pub position: f64,
    pub velocity: f64,
    /// True when the release velocity points towards increasing position.
    pub forward: bool,
}

/// 1:1 drag along one axis with grab offset and velocity history.
#[derive(Debug, Clone)]
pub struct DragGesture {
    axis: Axis,
    phase: DragPhase,
    start: (f64, f64),
    grab_offset: f64,
    position: f64,
    tracker: VelocityTracker,
}

impl DragGesture {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            phase: DragPhase::Idle,
            start: (0.0, 0.0),
            grab_offset: 0.0,
            position: 0.0,
            tracker: VelocityTracker::new(),
        }
    }

    pub fn phase(&self) -> DragPhase {
        self.phase
    }

    pub fn is_active(&self) -> bool {
        matches!(self.phase, DragPhase::Pending | DragPhase::Dragging)
    }

    pub fn is_dragging(&self) -> bool {
        self.phase == DragPhase::Dragging
    }

    pub fn grab_offset(&self) -> f64 {
        self.grab_offset
    }

    fn along(&self, x: f64, y: f64) -> f64 {
        match self.axis {
            Axis::Horizontal => x,
            Axis::Vertical => y,
        }
    }

    /// Starts tracking. `element_origin` is where the element is drawn now, so
    /// `grab_offset = pointer - origin` stops it snapping under the cursor.
    pub fn pointer_down(&mut self, x: f64, y: f64, element_origin: f64, time: f64) {
        let pointer = self.along(x, y);
        self.phase = DragPhase::Pending;
        self.start = (x, y);
        self.grab_offset = pointer - element_origin;
        self.position = element_origin;
        self.tracker.reset();
        self.tracker.push(element_origin, time);
    }

    /// Feeds a move. Returns the element's new origin while dragging.
    pub fn pointer_move(&mut self, x: f64, y: f64, time: f64) -> Option<f64> {
        match self.phase {
            DragPhase::Pending => {
                let (dx, dy) = ((x - self.start.0).abs(), (y - self.start.1).abs());
                if dx.max(dy) < DRAG_THRESHOLD {
                    return None;
                }
                let matches_axis = match self.axis {
                    Axis::Horizontal => dx >= dy,
                    Axis::Vertical => dy >= dx,
                };
                self.phase = if matches_axis { DragPhase::Dragging } else { DragPhase::Rejected };
                if !matches_axis {
                    return None;
                }
            }
            DragPhase::Dragging => {}
            DragPhase::Idle | DragPhase::Rejected => return None,
        }
        self.position = self.along(x, y) - self.grab_offset;
        self.tracker.push(self.position, time);
        Some(self.position)
    }

    /// Finishes the gesture. Commit versus reversal must come from the sign
    /// of the returned velocity, not from position alone.
    pub fn pointer_up(&mut self, time: f64) -> Option<Release> {
        let was_dragging = self.phase == DragPhase::Dragging;
        self.phase = DragPhase::Idle;
        was_dragging.then(|| {
            let velocity = self.tracker.velocity(time);
            Release { position: self.position, velocity, forward: velocity > 0.0 }
        })
    }

    /// Handles `pointercancel`, `lostpointercapture` and window blur. Any
    /// active drag ends with zero velocity so nothing stays stuck.
    pub fn cancel(&mut self) -> Option<Release> {
        let was_dragging = self.phase == DragPhase::Dragging;
        self.phase = DragPhase::Idle;
        self.tracker.reset();
        was_dragging.then_some(Release { position: self.position, velocity: 0.0, forward: false })
    }
}

/// Immediate press feedback with cancel-by-dragging-away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PressTracker {
    down: bool,
    inside: bool,
}

impl PressTracker {
    /// Visual pressed state. True on the exact frame of pointer down.
    pub fn is_pressed(&self) -> bool {
        self.down && self.inside
    }

    pub fn is_down(&self) -> bool {
        self.down
    }

    pub fn pointer_down(&mut self) {
        self.down = true;
        self.inside = true;
    }

    /// Updates engagement for a pointer offset relative to the target's
    /// bounds. Leaving the padded area disengages; returning re-engages.
    pub fn pointer_move(&mut self, x: f64, y: f64, width: f64, height: f64) {
        if self.down {
            self.inside = x >= -HIT_PADDING
                && y >= -HIT_PADDING
                && x <= width + HIT_PADDING
                && y <= height + HIT_PADDING;
        }
    }

    /// Returns true when the release counts as an activation.
    pub fn pointer_up(&mut self) -> bool {
        let activated = self.down && self.inside;
        *self = Self::default();
        activated
    }

    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_waits_for_threshold_then_tracks_with_grab_offset() {
        let mut drag = DragGesture::new(Axis::Horizontal);
        drag.pointer_down(500.0, 10.0, 480.0, 0.0);
        assert_eq!(drag.grab_offset(), 20.0);
        assert_eq!(drag.pointer_move(505.0, 10.0, 0.01), None);
        assert_eq!(drag.phase(), DragPhase::Pending);

        let position = drag.pointer_move(520.0, 10.0, 0.02).unwrap();
        assert_eq!(position, 500.0, "element must not jump to the cursor");
        assert!(drag.is_dragging());
    }

    #[test]
    fn perpendicular_motion_rejects_the_drag() {
        let mut drag = DragGesture::new(Axis::Horizontal);
        drag.pointer_down(0.0, 0.0, 0.0, 0.0);
        assert_eq!(drag.pointer_move(2.0, 30.0, 0.01), None);
        assert_eq!(drag.phase(), DragPhase::Rejected);
        assert_eq!(drag.pointer_move(80.0, 30.0, 0.02), None);
        assert_eq!(drag.pointer_up(0.03), None);
    }

    #[test]
    fn release_velocity_sign_decides_direction() {
        let mut drag = DragGesture::new(Axis::Horizontal);
        drag.pointer_down(0.0, 0.0, 0.0, 0.0);
        for i in 1..=8 {
            drag.pointer_move(i as f64 * 20.0, 0.0, i as f64 * 0.01);
        }
        // Reverse sharply at the end: position is far right, velocity is left.
        for i in 1..=10 {
            drag.pointer_move(160.0 - i as f64 * 5.0, 0.0, 0.08 + i as f64 * 0.01);
        }
        let release = drag.pointer_up(0.18).unwrap();
        assert!(release.position > 100.0);
        assert!(release.velocity < 0.0);
        assert!(!release.forward);
    }

    #[test]
    fn cancel_settles_without_velocity() {
        let mut drag = DragGesture::new(Axis::Vertical);
        drag.pointer_down(0.0, 0.0, 0.0, 0.0);
        drag.pointer_move(0.0, 40.0, 0.01);
        let release = drag.cancel().unwrap();
        assert_eq!(release.velocity, 0.0);
        assert!(!drag.is_active());
        assert_eq!(drag.cancel(), None, "second cancel is a no-op");
    }

    #[test]
    fn press_engages_on_down_and_survives_small_slip() {
        let mut press = PressTracker::default();
        press.pointer_down();
        assert!(press.is_pressed());
        press.pointer_move(105.0, 45.0, 100.0, 40.0);
        assert!(press.is_pressed(), "within the 10px padding");
        assert!(press.pointer_up());
    }

    #[test]
    fn press_cancels_when_dragged_away_and_reengages_on_return() {
        let mut press = PressTracker::default();
        press.pointer_down();
        press.pointer_move(200.0, 20.0, 100.0, 40.0);
        assert!(!press.is_pressed());
        assert!(press.is_down());
        press.pointer_move(50.0, 20.0, 100.0, 40.0);
        assert!(press.is_pressed());

        press.pointer_move(200.0, 20.0, 100.0, 40.0);
        assert!(!press.pointer_up(), "released outside: no activation");
    }

    #[test]
    fn press_cancel_clears_state() {
        let mut press = PressTracker::default();
        press.pointer_down();
        press.cancel();
        assert!(!press.is_pressed());
        assert!(!press.pointer_up());
    }
}
