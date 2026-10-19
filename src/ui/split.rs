//! A two-pane split with pixel-stable sizing and no layout-to-notification loop.
use super::{BORDER, HOVER};
use gpui_kit::base::ElementExt;
use gpui_kit::{
    AnyElement, App, Bounds, Context, ElementId, Entity, IntoElement, MouseButton, MouseMoveEvent,
    MouseUpEvent, Pixels, RenderOnce, TestSupportExt, Window, canvas, div, prelude::*, px, rgb,
};

const DIVIDER: Pixels = px(1.);

pub struct SplitState {
    preferred: Pixels,
    minimum_left: Pixels,
    minimum_right: Pixels,
    bounds: Bounds<Pixels>,
    drag_start: Option<(Pixels, Pixels)>,
}

impl SplitState {
    pub fn new(preferred: Pixels, minimum_left: Pixels, minimum_right: Pixels) -> Self {
        Self {
            preferred,
            minimum_left,
            minimum_right,
            bounds: Bounds::default(),
            drag_start: None,
        }
    }

    pub fn width(&self) -> Pixels {
        if self.bounds.size.width <= px(0.) {
            return self.preferred;
        }
        let maximum = (self.bounds.size.width - self.minimum_right - DIVIDER).max(px(0.));
        self.preferred
            .clamp(self.minimum_left.min(maximum), maximum)
    }

    pub fn resize(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = px(width.as_f32().round());
        let before = self.width();
        self.preferred = width;
        self.preferred = self.width();
        if self.width() != before {
            cx.notify();
        }
    }

    fn measure(&mut self, bounds: Bounds<Pixels>) -> bool {
        let before = self.width();
        self.bounds = bounds;
        self.width() != before
    }
}

#[derive(IntoElement)]
pub struct SplitPane {
    id: ElementId,
    state: Entity<SplitState>,
    left: AnyElement,
    right: AnyElement,
}

impl SplitPane {
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<SplitState>,
        left: impl IntoElement,
        right: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            left: left.into_any_element(),
            right: right.into_any_element(),
        }
    }
}

impl RenderOnce for SplitPane {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = self.state.read(cx).width();
        let dragging = self.state.read(cx).drag_start.is_some();
        let measure = self.state.clone();
        let start = self.state.clone();
        let pointer = self.state.clone();
        div()
            .id(self.id)
            .relative()
            .flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_prepaint(move |bounds, window, cx| {
                let changed = measure.update(cx, |state, _| state.measure(bounds));
                // Only a change to the effective pane width needs a settling frame.
                if changed {
                    window.defer(cx, move |_, cx| measure.update(cx, |_, cx| cx.notify()));
                }
            })
            .child(
                div()
                    .w(width)
                    .h_full()
                    .flex_shrink_0()
                    .min_w_0()
                    .min_h_0()
                    .child(self.left),
            )
            .child(
                div()
                    .relative()
                    .w(DIVIDER)
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(BORDER)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(self.right),
            )
            // A generous hit target without reserving eight pixels of workspace.
            .child(
                div()
                    .id("split-divider")
                    .test_support()
                    .absolute()
                    .left(width - px(3.))
                    .top_0()
                    .bottom_0()
                    .w(px(8.))
                    .cursor_col_resize()
                    .hover(|style| style.bg(rgb(HOVER)))
                    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                        cx.stop_propagation();
                        start.update(cx, |state, cx| {
                            state.drag_start = Some((event.position.x, state.width()));
                            cx.notify();
                        });
                    }),
            )
            .when(dragging, |view| {
                view.child(div().absolute().inset_0().cursor_col_resize())
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let moving = pointer.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if !phase.bubble() || moving.read(cx).drag_start.is_none() {
                                return;
                            }
                            moving.update(cx, |state, cx| {
                                if let Some((origin, width)) = state.drag_start {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        state.resize(width + event.position.x - origin, cx);
                                        cx.stop_propagation();
                                    } else {
                                        state.drag_start = None;
                                        cx.notify();
                                    }
                                }
                            });
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase.bubble()
                                && event.button == MouseButton::Left
                                && pointer.read(cx).drag_start.is_some()
                            {
                                pointer.update(cx, |state, cx| {
                                    if state.drag_start.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}
