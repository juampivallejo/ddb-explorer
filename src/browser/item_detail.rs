use crate::ui::{self, ITEM_BACKGROUND};
use gpui_kit::component::{
    IconName,
    button::{Button, ButtonVariants},
    input::{Editor, EditorState},
};
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, IntoElement, Render, SharedString, Window, div,
    prelude::*, rgb,
};
use serde_json::Value;

pub(super) struct ItemDetail {
    editor: Entity<EditorState>,
    item: Option<Value>,
    json: SharedString,
}

impl ItemDetail {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    .line_number(false)
                    .folding(false)
            }),
            item: None,
            json: SharedString::default(),
        }
    }

    pub fn set_item(&mut self, item: Option<&Value>, window: &mut Window, cx: &mut Context<Self>) {
        // Unrelated renders and in-flight reads must preserve selection and scroll.
        if self.item.as_ref() == item {
            return;
        }
        self.item = item.cloned();
        self.json = item
            .map(|item| serde_json::to_string_pretty(item).expect("JSON values serialize"))
            .unwrap_or_default()
            .into();
        self.editor.update(cx, |editor, cx| {
            editor.set_value(self.json.clone(), window, cx);
        });
        cx.notify();
    }
}

impl Render for ItemDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut detail = div().relative().size_full().min_w_0().min_h_0();
        if self.item.is_none() {
            return detail.child(ui::empty_state(
                "No matching items",
                "Try another key value or read the next page if available.",
            ));
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
                        .bg(rgb(ITEM_BACKGROUND))
                        .icon(IconName::Copy)
                        .tooltip("Copy JSON")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(this.json.to_string()));
                        })),
                ),
            );
        detail
    }
}
