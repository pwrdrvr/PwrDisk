//! Every scroll area's bar, and the edges that say there is more.
//!
//! GPUI's overflow scrolling, uniform lists and lists clip and scroll but
//! paint no bar. Without one a scroll area's extent is invisible, and a
//! control cut off at its edge looks covered by whatever lies below it.
//! Each scroll area draws [`bar`] in a relative frame around it, sharing
//! its handle; a frame with room beside the content puts the bar in that
//! gutter rather than over the content's buttons.

use gpui_kit::base::{
    Scrollbar, ScrollbarAxis, ScrollbarHandle, ScrollbarMode,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Stateful, Styled as _, div, px,
};

use crate::ui::space;

/// Where a scroll area stands against its content, from its last layout
/// and current offset: whether it scrolls at all, and whether there is
/// more out of sight above or below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub scrolls: bool,
    pub more_above: bool,
    pub more_below: bool,
}

impl Extent {
    pub fn of(handle: &impl ScrollbarHandle) -> Self {
        Self::from_measures(
            handle.viewport_bounds().size.height,
            handle.content_size().height,
            handle.offset().y,
        )
    }

    /// `offset` is GPUI's: zero at the top and negative as the content
    /// moves up.
    pub fn from_measures(
        viewport: Pixels,
        content: Pixels,
        offset: Pixels,
    ) -> Self {
        // Half a pixel: layout rounds, and a sub-pixel remainder is not
        // content anyone could scroll to.
        let slack = px(0.5);
        let max = content - viewport;
        let scrolls = max > slack;
        Self {
            scrolls,
            more_above: scrolls && offset < -slack,
            more_below: scrolls && offset > -max + slack,
        }
    }
}

/// The bar for the scroll area that `handle` tracks, drawn in the scroll
/// area's relative frame, `right` from the frame's right edge: zero
/// overlays the content's edge, a negative value puts the bar in a gutter
/// beside it. While the content overflows it is always shown, not only
/// mid-scroll, so the extent can be read at a glance; while it fits,
/// nothing is painted.
///
/// The element is always in the tree: GPUI's scrollbar decides whether to
/// paint from the layout of the frame it is painted in, after the scroll
/// area has been laid out. What render saw is a frame old, and an area's
/// first frame has no layout at all; when it disagrees with what was laid
/// out, the bar asks for one more frame, so the window's state (and the
/// tests' `id`, given only while it scrolls) catches up with no input.
pub fn bar<H: ScrollbarHandle + Clone>(
    id: &'static str,
    handle: &H,
    right: Pixels,
    rem: f32,
    ink: Hsla,
) -> Stateful<Div> {
    let shown = Extent::of(handle).scrolls;
    let laid_out = handle.clone();
    div()
        // `refresh` is ignored while a frame draws; a next-frame request
        // is not.
        .on_children_prepainted(move |_, window, _| {
            if Extent::of(&laid_out).scrolls != shown {
                window.request_animation_frame();
            }
        })
        .id(id)
        .when(shown, |bar| bar.debug_selector(move || id.into()))
        .absolute()
        .top_0()
        .bottom_0()
        .right(right)
        .w(px(0.5 * rem))
        .child(
            Scrollbar::new(handle)
                .id(ElementId::Name(format!("{id}-thumb").into()))
                // Painted where this frame is, not over the handle's
                // viewport, which is the default: a bar meant for the
                // gutter otherwise lands on the content's edge.
                .viewport_from_layout()
                .axis(ScrollbarAxis::Vertical)
                .mode(ScrollbarMode::Always)
                .styles(move |style| {
                    style
                        .track(|track| {
                            track.width(px(0.5 * rem)).bg(ink.opacity(0.06))
                        })
                        .thumb(|thumb| {
                            thumb
                                .width(px(0.375 * rem))
                                .inset(px(0.0625 * rem))
                                .radius(px(0.))
                                .bg(ink.opacity(0.35))
                        })
                        .thumb_hover(|thumb| thumb.bg(ink.opacity(0.55)))
                        .thumb_active(|thumb| thumb.bg(ink.opacity(0.7)))
                }),
        )
}

/// The `right` that centres [`bar`] in a gutter `width` wide outside the
/// frame's right edge, such as a panel's side padding.
pub fn in_gutter(width: Pixels, rem: f32) -> Pixels {
    -(width + px(0.5 * rem)) / 2.
}

/// The `right` that centres [`bar`] in the scroll area's own right-hand
/// padding, `width` wide, inside the frame.
pub fn in_padding(width: Pixels, rem: f32) -> Pixels {
    (width - px(0.5 * rem)) / 2.
}

/// A scroll area in the relative frame its bar is drawn in. `frame` takes
/// the scroll area's place in its parent's layout; the bar adds no size.
pub fn framed(frame: Div, scroll: impl IntoElement, bar: Stateful<Div>) -> Div {
    frame.relative().child(scroll).child(bar)
}

/// An edge of a scroll area with more beyond it: the content fades into
/// `ground` and a hairline runs `bleed` past the frame on either side, so
/// what is cut off reads as scrolled out of sight, not as covered by what
/// lies beyond. It takes no clicks: the content under the fade stays live.
pub fn edge(
    id: &'static str,
    top: bool,
    bleed: Pixels,
    line: Hsla,
    ground: Hsla,
) -> Stateful<Div> {
    // From the edge inward: 180° runs top to bottom, 0° bottom to top.
    let angle = if top { 180. } else { 0. };
    let hairline = div()
        .absolute()
        .left(-bleed)
        .right(-bleed)
        .h(px(1.))
        .bg(line)
        .map(|rule| if top { rule.top_0() } else { rule.bottom_0() });
    div()
        .id(id)
        .debug_selector(move || id.into())
        .absolute()
        .left_0()
        .right_0()
        .h(space::XL)
        .map(|fade| if top { fade.top_0() } else { fade.bottom_0() })
        .bg(gpui_kit::linear_gradient(
            angle,
            gpui_kit::linear_color_stop(ground, 0.),
            gpui_kit::linear_color_stop(ground.opacity(0.), 1.),
        ))
        .child(hairline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        Context, Render, ScrollHandle, StatefulInteractiveElement as _,
        TestAppContext, Window,
    };

    /// A scroll area taller than its frame, with its bar, and nothing
    /// else that could ask for a frame.
    struct Area {
        scroll: ScrollHandle,
        rows: usize,
    }

    impl Render for Area {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut Context<'_, Self>,
        ) -> impl IntoElement {
            framed(
                div().size_full(),
                div()
                    .id("area")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children((0..self.rows).map(|_| div().h(px(20.)))),
                bar("area-bar", &self.scroll, px(0.), 16., gpui_kit::black()),
            )
        }
    }

    /// Property: an area that overflows shows its bar on the frame after
    /// its first, with no input: render sees the last frame's layout, and
    /// a first frame has none. The bar was once left out of the tree until
    /// something else redrew, such as the pointer crossing a button.
    #[gpui_kit::test]
    fn a_bar_asks_for_the_frame_that_shows_it(cx: &mut TestAppContext) {
        cx.update(gpui_kit::base::init);
        let (view, cx) = cx.add_window_view(|_, _| Area {
            scroll: ScrollHandle::new(),
            rows: 2,
        });
        cx.simulate_resize(gpui_kit::size(px(200.), px(200.)));
        let draw = |cx: &mut gpui_kit::VisualTestContext| {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        };
        let asked = |cx: &mut gpui_kit::VisualTestContext| {
            cx.update(gpui_kit::Window::simulate_next_frame)
        };
        draw(cx);
        draw(cx);
        assert_eq!(asked(cx), 0, "two rows fit, and nothing moves");
        // The content grows past the frame: render still sees the layout
        // in which it fit.
        view.update(cx, |area, cx| {
            area.rows = 50;
            cx.notify();
        });
        draw(cx);
        assert!(asked(cx) > 0, "the frame asks for the next");
        draw(cx);
        assert!(cx.debug_bounds("area-bar").is_some(), "which shows it");
        assert_eq!(asked(cx), 0, "and then rests");
    }

    /// Model: an area scrolls exactly when its content is taller than its
    /// viewport by more than rounding, and each edge has more beyond it
    /// exactly when the offset has not reached that end.
    #[test]
    fn the_extent_follows_the_offset_between_its_ends() {
        let viewport = px(100.);
        // Each scrolling case moves at least a pixel a step, past rounding.
        for content in [0., 50., 100., 100.4, 108., 150., 400.] {
            let content = px(content);
            let max = (content - viewport).max(px(0.));
            for step in 0..=8_u8 {
                let offset = -max * (f32::from(step) / 8.);
                let extent = Extent::from_measures(viewport, content, offset);
                let scrolls = max > px(0.5);
                let case = format!("{content:?} at {offset:?}");
                assert_eq!(extent.scrolls, scrolls, "{case}");
                assert_eq!(
                    extent.more_above,
                    scrolls && step > 0,
                    "above, {case}"
                );
                assert_eq!(
                    extent.more_below,
                    scrolls && step < 8,
                    "below, {case}"
                );
            }
        }
    }
}
