//! Wheel, trackpad and pinch zoom on the mosaic, as a gesture.
//!
//! Zoom follows how far the input travelled, not how many events it came
//! in: a trackpad sends dozens of small events a second where a mouse sends
//! one per notch, and a fixed step per event made a light touch jump from
//! one end of the zoom to the other.
//!
//! Crossing a level is a push. Input past the zoom's limit (the directory
//! under the pointer filling the view, or 1× going out) fills a bar instead
//! of moving the view; a full bar opens or leaves one level, and the rest
//! of that gesture is ignored, so one swipe never runs through several
//! levels. Momentum after the fingers lift may zoom but never pushes.

use std::time::{Duration, Instant};

/// Trackpad travel, in logical pixels, that doubles or halves the zoom.
pub const PIXELS_PER_DOUBLING: f32 = 240.0;
/// Wheel lines that double or halve it: three notches, about 1.26× a
/// notch. Windows reports a notch as the system's lines-per-notch setting,
/// three by default, where macOS and Linux report one line.
#[cfg(not(windows))]
pub const LINES_PER_DOUBLING: f32 = 3.0;
#[cfg(windows)]
pub const LINES_PER_DOUBLING: f32 = 9.0;
/// Travel past the limit, in doublings, that crosses a level: half a
/// doubling, 120 px on a trackpad or about two notches. Long enough that
/// reaching the limit never crosses by accident; short enough to be a
/// flick, not a chore.
pub const PUSH_DOUBLINGS: f32 = 0.5;
/// Input this far apart is two gestures. macOS streams momentum events at
/// the frame rate after the fingers lift, with no phase of their own, so a
/// pause is what ends a gesture that has crossed a level.
pub const GESTURE_GAP: Duration = Duration::from_millis(300);

/// Where a touch is, as the platform reports it. A wheel without phases
/// reports every event as `Moved`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
}

impl From<gpui_kit::TouchPhase> for Phase {
    fn from(phase: gpui_kit::TouchPhase) -> Self {
        match phase {
            gpui_kit::TouchPhase::Started => Self::Started,
            gpui_kit::TouchPhase::Moved => Self::Moved,
            // A cancelled touch has lifted too.
            gpui_kit::TouchPhase::Ended | gpui_kit::TouchPhase::Cancelled => {
                Self::Ended
            }
        }
    }
}

/// Which limit a push is against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Zooming in, at the directory under the pointer: opens it.
    In,
    /// Zooming out, at 1×: goes up a level.
    Out,
}

/// How far the view may still zoom each way from where it is, in
/// doublings, and whether there is a level past each limit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Room {
    pub zoom_in: f32,
    pub zoom_out: f32,
    pub can_open: bool,
    pub can_leave: bool,
}

/// What one event does: zoom by `zoom` doublings (negative is out), then
/// cross a level if `cross` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    pub zoom: f32,
    pub cross: Option<Edge>,
}

/// One gesture's state.
#[derive(Clone, Debug, Default)]
pub struct Gesture {
    last: Option<Instant>,
    /// The fingers lifted: what follows is momentum.
    coasting: bool,
    /// A level changed during this gesture.
    spent: bool,
    /// How full the push bar is, `0..1`, and against which limit.
    push: f32,
    edge: Option<Edge>,
}

impl Gesture {
    /// Feed one event of `doublings` travel (positive zooms in).
    pub fn feed(
        &mut self,
        doublings: f32,
        phase: Phase,
        now: Instant,
        room: Room,
    ) -> Outcome {
        let fresh = phase == Phase::Started
            || self.last.is_none_or(|last| {
                now.saturating_duration_since(last) >= GESTURE_GAP
            });
        if fresh {
            *self = Self::default();
        }
        self.last = Some(now);
        // This event is the fingers' own; only what follows it coasts.
        let coasting = self.coasting;
        if phase == Phase::Ended {
            self.coasting = true;
        }
        if self.spent || !doublings.is_finite() || doublings == 0.0 {
            return Outcome::default();
        }

        let (edge, travel, room_left, beyond) = if doublings > 0.0 {
            (Edge::In, doublings, room.zoom_in.max(0.0), room.can_open)
        } else {
            (
                Edge::Out,
                -doublings,
                room.zoom_out.max(0.0),
                room.can_leave,
            )
        };
        // Reversing empties the bar: backing off is never half a crossing.
        if self.edge.is_some_and(|pushing| pushing != edge) {
            self.clear_push();
        }
        let moved = travel.min(room_left);
        let zoom = if edge == Edge::In { moved } else { -moved };
        let rest = travel - moved;
        if rest <= f32::EPSILON || !beyond || coasting {
            return Outcome { zoom, cross: None };
        }
        self.edge = Some(edge);
        self.push += rest / PUSH_DOUBLINGS;
        if self.push < 1.0 {
            return Outcome { zoom, cross: None };
        }
        self.clear_push();
        self.spent = true;
        Outcome {
            zoom,
            cross: Some(edge),
        }
    }

    /// The bar to draw: which limit, and how full.
    pub fn cue(&self) -> Option<(Edge, f32)> {
        self.edge
            .filter(|_| self.push > 0.0)
            .map(|edge| (edge, self.push.min(1.0)))
    }

    /// A pause empties the bar. Returns whether it was showing.
    pub fn expire(&mut self, now: Instant) -> bool {
        let quiet = self.last.is_none_or(|last| {
            now.saturating_duration_since(last) >= GESTURE_GAP
        });
        if quiet && self.cue().is_some() {
            self.clear_push();
            return true;
        }
        false
    }

    /// Forget the gesture: the view changed under it some other way.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Empty the bar but keep the gesture: what it was pushing toward
    /// changed, so the push so far no longer counts.
    pub const fn drop_push(&mut self) {
        self.clear_push();
    }

    const fn clear_push(&mut self) {
        self.push = 0.0;
        self.edge = None;
    }
}

/// A wheel or trackpad event's vertical travel as doublings, positive in.
pub fn doublings(delta: gpui_kit::ScrollDelta) -> f32 {
    match delta {
        gpui_kit::ScrollDelta::Pixels(delta) => {
            delta.y.as_f32() / PIXELS_PER_DOUBLING
        }
        gpui_kit::ScrollDelta::Lines(delta) => delta.y / LINES_PER_DOUBLING,
    }
}

/// A pinch's magnification as doublings: macOS reports `0.1` for 10%
/// larger.
pub fn pinch_doublings(magnification: f32) -> f32 {
    (1.0 + magnification).max(0.01).log2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ROOMY: Room = Room {
        zoom_in: 100.0,
        zoom_out: 100.0,
        can_open: true,
        can_leave: true,
    };

    /// Feed a run of events a frame apart, as a trackpad sends them, and
    /// follow the room as the view zooms. Returns the zoom travelled and the
    /// levels crossed.
    fn run(
        gesture: &mut Gesture,
        start: Instant,
        events: &[(f32, Phase)],
        mut room: Room,
    ) -> (f32, Vec<Edge>) {
        let mut zoomed = 0.0;
        let mut crossed = Vec::new();
        for (index, &(travel, phase)) in events.iter().enumerate() {
            let now = start + Duration::from_millis(16 * index as u64);
            let outcome = gesture.feed(travel, phase, now, room);
            zoomed += outcome.zoom;
            room.zoom_in -= outcome.zoom;
            room.zoom_out += outcome.zoom;
            if let Some(edge) = outcome.cross {
                crossed.push(edge);
                // A new level starts at 1× with its own room in.
                room = Room {
                    zoom_in: 1.5,
                    zoom_out: 0.0,
                    ..room
                };
            }
        }
        (zoomed, crossed)
    }

    fn split(total: f32, parts: usize) -> Vec<(f32, Phase)> {
        let each = total / parts as f32;
        (0..parts)
            .map(|index| {
                let phase = match index {
                    0 => Phase::Started,
                    _ if index + 1 == parts => Phase::Ended,
                    _ => Phase::Moved,
                };
                (each, phase)
            })
            .collect()
    }

    proptest! {
        /// The rule the fixed 1.15× step broke: the zoom is a function of
        /// how far the input travelled, however many events carried it.
        #[test]
        fn zoom_follows_travel_not_event_count(
            pixels in -600.0f32..600.0,
            parts in 1usize..80,
        ) {
            let total = pixels / PIXELS_PER_DOUBLING;
            let (zoomed, crossed) = run(
                &mut Gesture::default(),
                Instant::now(),
                &split(total, parts),
                ROOMY,
            );
            prop_assert!((zoomed - total).abs() < 1e-3, "{zoomed} vs {total}");
            prop_assert!(crossed.is_empty());
        }

        /// The rule per-event crossing broke: however long a gesture runs
        /// and however it turns, it crosses at most one level. Momentum
        /// streams on after the fingers lift without a pause.
        #[test]
        fn one_gesture_crosses_at_most_one_level(
            travel in prop::collection::vec(-0.5f32..0.5, 1..200),
            room_in in 0.0f32..2.0,
            room_out in 0.0f32..2.0,
        ) {
            let mut events: Vec<(f32, Phase)> =
                travel.iter().map(|&t| (t, Phase::Moved)).collect();
            events[0].1 = Phase::Started;
            let lift = events.len() / 2;
            events[lift].1 = Phase::Ended;
            let room = Room {
                zoom_in: room_in,
                zoom_out: room_out,
                can_open: true,
                can_leave: true,
            };
            let (_, crossed) =
                run(&mut Gesture::default(), Instant::now(), &events, room);
            prop_assert!(crossed.len() <= 1, "{crossed:?}");
        }

        /// The zoom never runs past either limit.
        #[test]
        fn the_zoom_stays_inside_its_room(
            travel in prop::collection::vec(-1.0f32..1.0, 1..60),
            room_in in 0.0f32..3.0,
            room_out in 0.0f32..3.0,
        ) {
            let mut gesture = Gesture::default();
            let start = Instant::now();
            let room = Room {
                zoom_in: room_in,
                zoom_out: room_out,
                can_open: false,
                can_leave: false,
            };
            let (zoomed, crossed) = run(
                &mut gesture,
                start,
                &travel.iter().map(|&t| (t, Phase::Moved)).collect::<Vec<_>>(),
                room,
            );
            prop_assert!(zoomed <= room_in + 1e-4);
            prop_assert!(-zoomed <= room_out + 1e-4);
            prop_assert!(crossed.is_empty(), "nothing past either limit");
        }
    }

    #[test]
    fn a_push_past_the_limit_crosses_after_half_a_doubling() {
        let mut gesture = Gesture::default();
        let now = Instant::now();
        let room = Room {
            zoom_in: 0.0,
            ..ROOMY
        };
        let outcome = gesture.feed(0.3, Phase::Started, now, room);
        assert_eq!(outcome.cross, None);
        let (edge, progress) = gesture.cue().expect("the bar shows");
        assert_eq!(edge, Edge::In);
        assert!((progress - 0.6).abs() < 1e-4, "{progress}");
        let outcome = gesture.feed(0.25, Phase::Moved, now, room);
        assert_eq!(outcome.cross, Some(Edge::In));
        assert_eq!(gesture.cue(), None);
        // The rest of the gesture, momentum included, does nothing.
        let later = now + Duration::from_millis(100);
        assert_eq!(
            gesture.feed(5.0, Phase::Moved, later, ROOMY),
            Outcome::default()
        );
    }

    #[test]
    fn momentum_zooms_but_never_pushes() {
        let mut gesture = Gesture::default();
        let now = Instant::now();
        let room = Room {
            zoom_in: 0.1,
            ..ROOMY
        };
        gesture.feed(0.0, Phase::Ended, now, room);
        let outcome = gesture.feed(2.0, Phase::Moved, now, room);
        assert!((outcome.zoom - 0.1).abs() < 1e-6);
        assert_eq!(outcome.cross, None);
        assert_eq!(gesture.cue(), None);
    }

    #[test]
    fn reversing_or_pausing_empties_the_bar() {
        let mut gesture = Gesture::default();
        let now = Instant::now();
        let room = Room {
            zoom_in: 0.0,
            ..ROOMY
        };
        gesture.feed(0.2, Phase::Started, now, room);
        assert!(gesture.cue().is_some());
        gesture.feed(-0.01, Phase::Moved, now, room);
        assert_eq!(gesture.cue(), None);

        gesture.feed(0.2, Phase::Moved, now, room);
        assert!(!gesture.expire(now + Duration::from_millis(100)));
        assert!(gesture.cue().is_some());
        assert!(gesture.expire(now + GESTURE_GAP));
        assert_eq!(gesture.cue(), None);
    }

    #[test]
    fn a_new_touch_or_a_pause_starts_a_new_gesture() {
        let room = Room {
            zoom_in: 0.0,
            ..ROOMY
        };
        let now = Instant::now();
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(1.0, Phase::Started, now, room).cross,
            Some(Edge::In)
        );
        assert_eq!(
            gesture.feed(1.0, Phase::Started, now, room).cross,
            Some(Edge::In)
        );

        let mut gesture = Gesture::default();
        gesture.feed(1.0, Phase::Moved, now, room);
        assert_eq!(gesture.feed(1.0, Phase::Moved, now, room).cross, None);
        let later = now + GESTURE_GAP;
        assert_eq!(
            gesture.feed(1.0, Phase::Moved, later, room).cross,
            Some(Edge::In)
        );
    }

    #[test]
    fn units_match_the_promised_travel() {
        use gpui_kit::{ScrollDelta, point, px};
        let pixels = ScrollDelta::Pixels(point(px(0.), px(240.)));
        assert!((doublings(pixels) - 1.0).abs() < 1e-6);
        let notch =
            ScrollDelta::Lines(point(0., if cfg!(windows) { 3. } else { 1. }));
        assert!((doublings(notch).exp2() - 1.26).abs() < 0.01);
        assert!((pinch_doublings(1.0) - 1.0).abs() < 1e-6);
    }
}
