use super::{ItemBrowser, query::ReadMode};
use crate::ui::{self, BORDER, HOVER, MUTED, PANEL, SELECTED};
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{ClipboardItem, Context, Div, Window, div, prelude::*, rgb};

fn key_row(
    index: usize,
    kind: &'static str,
    name: &str,
    value: &serde_json::Value,
    cx: &mut Context<ItemBrowser>,
) -> Div {
    let copy_value = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_ellipsis()
                .child(value.to_string()),
        )
        .child(
            Button::new((kind, index))
                .ghost()
                .small()
                .flex_shrink_0()
                .icon(IconName::Copy)
                .tooltip(format!("Copy {name}"))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_value.clone()));
                })),
        )
}

impl ItemBrowser {
    pub(super) fn results(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let selected = self.state.reads.displayed.as_ref().and_then(|displayed| {
            displayed.selected_item.and_then(|index| {
                displayed
                    .page
                    .items
                    .get(index)
                    .map(|item| ((displayed.id, index), item))
            })
        });
        self.detail
            .update(cx, |detail, cx| detail.set_item(selected, window, cx));
        let Some(displayed) = &self.state.reads.displayed else {
            // An empty state describes a settled view, not a request in flight.
            if self.state.schema.is_loading()
                || self.state.reads.is_loading()
                || self.error().is_some()
            {
                return div().flex_1();
            }
            return div().flex_1().child(ui::empty_state(
                "No items loaded",
                "Run a query or prepare a scan to read a page.",
            ));
        };
        let page = &displayed.page;
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .min_h_16()
            .p_3()
            .child(div().flex_1().min_w_0().text_sm().child(format!(
                "{} items in this page · {}",
                page.items.len(),
                displayed.request.summary()
            )));
        if page.next_key.is_some() && !self.state.reads.is_loading() {
            header = header.child(
                ui::choice(
                    "next-page",
                    if self.mode == ReadMode::Scan {
                        "Prepare next scan page…"
                    } else {
                        "Next query page"
                    },
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(pending) = this.state.reads.next_page() {
                        this.submit(pending, cx);
                    }
                    cx.notify();
                })),
            );
        }
        let list = div()
            .id("items-list")
            .flex()
            .flex_col()
            .w_64()
            .flex_shrink_0()
            .overflow_y_scroll()
            .gap_2()
            .p_3()
            .border_r_1()
            .border_color(rgb(BORDER))
            .children(page.items.iter().enumerate().map(|(index, item)| {
                let target = &displayed.request.target;
                let title = match item.get(&target.partition.name) {
                    Some(value) => key_row(
                        index,
                        "copy-partition-key",
                        &target.partition.name,
                        value,
                        cx,
                    ),
                    None => div().child(format!("Item {}", index + 1)),
                };
                let sort = target
                    .sort
                    .as_ref()
                    .and_then(|key| item.get(&key.name).map(|value| (key, value)));
                div()
                    .id(("item", index))
                    .p_2()
                    .rounded_md()
                    .bg(rgb(if displayed.selected_item == Some(index) {
                        SELECTED
                    } else {
                        PANEL
                    }))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(title)
                    .when_some(sort, |item, (key, value)| {
                        item.child(
                            key_row(index, "copy-sort-key", &key.name, value, cx)
                                .text_sm()
                                .text_color(rgb(MUTED)),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.reads.select_item(index);
                        cx.notify();
                    }))
            }));
        let detail = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.detail.clone());
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(div().flex().flex_1().min_h_0().child(list).child(detail))
    }
}
