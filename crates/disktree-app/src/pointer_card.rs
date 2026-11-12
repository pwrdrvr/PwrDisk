//! A card that follows the pointer and is placed by its measured size.
//!
//! The treemap's hover card used to be placed from a guessed height, so a
//! card whose path or clone split wrapped over more lines than the guess
//! ran off the bottom of the window. [`PointerCard`] lays its child out
//! first and only then decides where it goes, with [`place`].

use gpui_kit::{
    AnyElement, App, Bounds, Display, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Position, Size,
    Style, Window, point,
};

/// Where a `card` sized card goes beside `pointer`, kept inside `area`.
///
/// Below and right of the pointer by `gap`, as a native cursor tooltip
/// sits, flipped to the other side on an axis where that would overflow.
/// Flipping, not sliding, so the card never moves under the pointer while
/// either side has room. Only when neither side does is the card clamped
/// into `area` on that axis; it then covers the pointer's row or column,
/// but the other axis still keeps it beside the tip. A card larger than
/// `area` keeps its top-left corner in it, where its title is.
pub fn place(
    pointer: Point<Pixels>,
    card: Size<Pixels>,
    area: Bounds<Pixels>,
    gap: Pixels,
) -> Point<Pixels> {
    point(
        along(pointer.x, card.width, area.left(), area.right(), gap),
        along(pointer.y, card.height, area.top(), area.bottom(), gap),
    )
}

/// [`place`] on one axis: the card's start, `length` long, in `[low, high]`.
fn along(
    pointer: Pixels,
    length: Pixels,
    low: Pixels,
    high: Pixels,
    gap: Pixels,
) -> Pixels {
    let after = pointer + gap;
    if after + length <= high {
        return after;
    }
    let before = pointer - gap - length;
    if before >= low {
        return before;
    }
    // Neither side fits: slide in from the end, and never past the start.
    after.min(high - length).max(low)
}

/// A child laid out at its own size, then moved to [`place`] it beside a
/// pointer in window coordinates, inside `area`, also in window coordinates.
pub struct PointerCard {
    child: AnyElement,
    pointer: Point<Pixels>,
    area: Bounds<Pixels>,
    gap: Pixels,
}

impl PointerCard {
    pub fn new(
        child: impl IntoElement,
        pointer: Point<Pixels>,
        area: Bounds<Pixels>,
        gap: Pixels,
    ) -> Self {
        Self {
            child: child.into_any_element(),
            pointer,
            area,
            gap,
        }
    }
}

impl IntoElement for PointerCard {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PointerCard {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(
        &self,
    ) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.child.request_layout(window, cx);
        // Absolute, so the card takes no room from the layout it floats
        // over; flex, so the child keeps its own width.
        let style = Style {
            position: Position::Absolute,
            display: Display::Flex,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = window.layout_bounds(*child).size;
        let origin = place(self.pointer, size, self.area, self.gap);
        // Rounded so the card's border lands on whole pixels.
        let offset = origin - bounds.origin;
        let offset = point(offset.x.round(), offset.y.round());
        window.with_element_offset(offset, |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, size};
    use proptest::prelude::*;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn the_card_sits_below_and_right_of_the_pointer_when_it_fits() {
        let at = place(
            point(px(100.), px(100.)),
            size(px(200.), px(100.)),
            bounds(0., 0., 800., 600.),
            px(12.),
        );
        assert_eq!(at, point(px(112.), px(112.)));
    }

    #[test]
    fn a_card_near_the_corner_flips_above_and_left() {
        let at = place(
            point(px(790.), px(590.)),
            size(px(200.), px(100.)),
            bounds(0., 0., 800., 600.),
            px(12.),
        );
        assert_eq!(at, point(px(578.), px(478.)));
    }

    proptest! {
        /// A card that fits the area is drawn wholly inside it, wherever
        /// the pointer is, and beside the pointer, never on it, whenever
        /// either axis has room for the card on one side of the tip.
        #[test]
        fn a_card_that_fits_stays_inside_and_off_the_pointer(
            area_x in -400.0_f32..400.,
            area_y in -400.0_f32..400.,
            area_w in 40.0_f32..2000.,
            area_h in 40.0_f32..2000.,
            px_frac in 0.0_f32..=1.,
            py_frac in 0.0_f32..=1.,
            w_frac in 0.0_f32..=1.,
            h_frac in 0.0_f32..=1.,
            gap in 0.0_f32..30.,
        ) {
            let area = bounds(area_x, area_y, area_w, area_h);
            let pointer = point(
                px(area_w.mul_add(px_frac, area_x)),
                px(area_h.mul_add(py_frac, area_y)),
            );
            let card = size(px(area_w * w_frac), px(area_h * h_frac));
            let card_at = Bounds::new(place(pointer, card, area, px(gap)), card);
            // A thousandth of a pixel for the float sums of the edges.
            let slack = px(1e-3);
            prop_assert!(
                card_at.left() >= area.left() - slack
                    && card_at.top() >= area.top() - slack
                    && card_at.right() <= area.right() + slack
                    && card_at.bottom() <= area.bottom() + slack,
                "{card_at:?} leaves {area:?}"
            );
            let room = |p: Pixels, len: Pixels, lo: Pixels, hi: Pixels| {
                p + px(gap) + len <= hi || p - px(gap) - len >= lo
            };
            let beside = room(pointer.x, card.width, area.left(), area.right())
                || room(pointer.y, card.height, area.top(), area.bottom());
            // Strictly inside: a zero gap may leave the card's edge on it.
            let covers = card_at.left() < pointer.x
                && pointer.x < card_at.right()
                && card_at.top() < pointer.y
                && pointer.y < card_at.bottom();
            prop_assert!(
                !(beside && covers),
                "{card_at:?} covers the pointer at {pointer:?}"
            );
        }

        /// A card larger than the area keeps its top-left in it.
        #[test]
        fn an_oversized_card_keeps_its_corner_in_the_area(
            pointer_x in 0.0_f32..400.,
            pointer_y in 0.0_f32..300.,
            extra_w in 1.0_f32..400.,
            extra_h in 1.0_f32..400.,
        ) {
            let area = bounds(0., 0., 400., 300.);
            let card = size(px(400. + extra_w), px(300. + extra_h));
            let at = place(point(px(pointer_x), px(pointer_y)), card, area, px(8.));
            prop_assert_eq!(at, area.origin);
        }
    }
}
