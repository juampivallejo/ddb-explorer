use crate::ui::{self, ITEM_BACKGROUND, MUTED};
use gpui_kit::component::{
    Disableable, IconName,
    button::{Button, ButtonVariants},
    input::{Editor, EditorState},
};
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, IntoElement, Render, SharedString, Task, Window,
    div, prelude::*, rgb,
};
use serde_json::Value;

pub(super) struct ItemDetail {
    editor: Entity<EditorState>,
    selection: Option<(u64, usize)>,
    json: Option<SharedString>,
    ready: bool,
    format_task: Option<Task<()>>,
}

impl ItemDetail {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    // Wrapping eagerly shapes the whole document on the UI thread.
                    // Long DynamoDB strings use horizontal scrolling instead.
                    .soft_wrap(false)
                    .line_number(true)
                    .folding(true)
            }),
            selection: None,
            json: None,
            ready: false,
            format_task: None,
        }
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.format_task = None;
        self.selection = None;
        self.json = None;
        self.ready = false;
        cx.notify();
    }

    pub fn set_item(
        &mut self,
        item: Option<((u64, usize), &Value)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Unrelated renders and in-flight reads must preserve selection and scroll.
        let selection = item.map(|(selection, _)| selection);
        if self.selection == selection {
            return;
        }
        self.format_task = None;
        self.selection = selection;
        self.ready = false;
        let Some((selection, item)) = item else {
            self.clear(cx);
            return;
        };
        // Keep the previous document visible until formatting finishes, avoiding
        // an empty frame between clicks. Copy is disabled during this transition.
        let item = item.clone();
        let formatted = cx.background_executor().spawn(async move {
            SharedString::from(serde_json::to_string_pretty(&item).expect("JSON values serialize"))
        });
        self.format_task = Some(cx.spawn_in(window, async move |this, cx| {
            let json = formatted.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // A later click or mode change must never install an old item's JSON.
                if this.selection != Some(selection) {
                    return;
                }
                this.editor.update(cx, |editor, cx| {
                    editor.set_value(json.clone(), window, cx);
                });
                this.json = Some(json);
                this.ready = true;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for ItemDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut detail = div().relative().size_full().min_w_0().min_h_0();
        if self.selection.is_none() {
            return detail.child(ui::empty_state(
                "No matching items",
                "Try another key value or read the next page if available.",
            ));
        }
        if self.json.is_none() {
            return detail.bg(rgb(ITEM_BACKGROUND)).child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Preparing item…"),
            );
        }
        detail = detail
            .bg(rgb(ITEM_BACKGROUND))
            .child(
                Editor::new(&self.editor)
                    .readonly(true)
                    .bordered(false)
                    .size_full()
                    .font_family("monospace")
                    .text_sm(),
            )
            .child(
                div().absolute().top_2().right_3().child(
                    Button::new("copy-json")
                        .ghost()
                        .disabled(!self.ready)
                        .bg(rgb(ITEM_BACKGROUND))
                        .icon(IconName::Copy)
                        .tooltip("Copy JSON")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.ready
                                && let Some(json) = &this.json
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(json.to_string()));
                            }
                        })),
                ),
            );
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    fn verbose_item() -> Value {
        serde_json::json!({
            "id": "large-item",
            "sections": (0..1000).map(|index| serde_json::json!({
                "index": index,
                "text": "A verbose paragraph with enough text to wrap in a narrow editor. ".repeat(4),
            })).collect::<Vec<_>>(),
        })
    }

    #[gpui_kit::test]
    fn large_items_format_off_thread_and_repeated_renders_preserve_the_editor(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (detail, visual) = cx.add_window_view(ItemDetail::new);
        let item = verbose_item();
        VisualTestContext::update(visual, |window, cx| {
            detail.update(cx, |detail, cx| {
                detail.set_item(Some(((1, 0), &item)), window, cx);
                // Formatting has been scheduled; the click handler didn't serialize it.
                assert!(detail.json.is_none());
            });
        });
        visual.run_until_parked();
        VisualTestContext::update(visual, |window, cx| {
            detail.update(cx, |detail, cx| {
                let json = detail.json.clone().unwrap();
                assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), item);
                detail.editor.update(cx, |editor, cx| {
                    editor.set_cursor_position(
                        gpui_kit::base::input::Position::new(7, 0),
                        window,
                        cx,
                    );
                });
                let cursor = detail.editor.read(cx).cursor();
                detail.set_item(Some(((1, 0), &item)), window, cx);
                assert_eq!(detail.json.as_ref(), Some(&json));
                assert_eq!(detail.editor.read(cx).value().as_ref(), json.as_ref());
                assert_eq!(detail.editor.read(cx).cursor(), cursor);
            });
            // Exercise real editor layout with the large document.
            window.draw(cx).clear(cx);
        });
    }

    #[gpui_kit::test]
    fn changing_selection_or_clearing_cancels_old_formatting(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (detail, visual) = cx.add_window_view(ItemDetail::new);
        let large = verbose_item();
        let next = serde_json::json!({"id": "next"});
        VisualTestContext::update(visual, |window, cx| {
            detail.update(cx, |detail, cx| {
                detail.set_item(Some(((1, 0), &large)), window, cx);
                detail.set_item(Some(((1, 1), &next)), window, cx);
            });
        });
        visual.run_until_parked();
        visual.read(|cx| {
            let json = detail.read(cx).json.as_ref().unwrap();
            assert_eq!(serde_json::from_str::<Value>(json).unwrap(), next);
        });
        // The same row index in a new page represents a different item.
        let refreshed = serde_json::json!({"id": "refreshed"});
        VisualTestContext::update(visual, |window, cx| {
            detail.update(cx, |detail, cx| {
                detail.set_item(Some(((2, 1), &refreshed)), window, cx);
                assert!(!detail.ready);
                assert_eq!(
                    serde_json::from_str::<Value>(detail.json.as_ref().unwrap()).unwrap(),
                    next
                );
            });
        });
        visual.run_until_parked();
        visual.read(|cx| {
            assert!(detail.read(cx).ready);
            assert_eq!(
                serde_json::from_str::<Value>(detail.read(cx).json.as_ref().unwrap()).unwrap(),
                refreshed
            );
        });
        VisualTestContext::update(visual, |window, cx| {
            detail.update(cx, |detail, cx| {
                detail.set_item(Some(((3, 0), &large)), window, cx);
                detail.clear(cx);
            });
        });
        visual.run_until_parked();
        visual.read(|cx| {
            assert!(detail.read(cx).selection.is_none());
            assert!(detail.read(cx).json.is_none());
        });
    }
}
