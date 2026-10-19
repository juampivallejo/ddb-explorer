use super::ItemBrowser;
use crate::ui::split::SplitPane;
use crate::ui::{self, BORDER, HOVER, MUTED, PANEL, SELECTED};
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
    scroll::Scrollbar,
    tooltip::Tooltip,
};
use gpui_kit::{
    ClipboardItem, Context, Div, TestSupportExt, Window, canvas, div, list, prelude::*, px, rgb,
};

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
    let tooltip_value = format!("{name}: {copy_value}");
    div()
        .flex()
        .items_start()
        .min_w_0()
        .gap_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .id(format!("key-value:{kind}:{index}"))
                .test_support()
                .whitespace_normal()
                .child(value.to_string())
                .tooltip(move |window, cx| Tooltip::new(tooltip_value.clone()).build(window, cx)),
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
    pub(super) fn results(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
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
            .flex_wrap()
            .flex_shrink_0()
            .justify_between()
            .gap_3()
            .min_h_16()
            .p_3()
            .child(
                div()
                    .id("executed-query-summary")
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_ellipsis()
                    .child(format!(
                        "{} items · Results for: {}",
                        page.items.len(),
                        displayed.request.summary()
                    ))
                    .tooltip({
                        let summary = displayed.request.summary();
                        move |window, cx| Tooltip::new(summary.clone()).build(window, cx)
                    }),
            );
        if matches!(
            displayed.request.operation,
            crate::dynamodb::ReadOperation::Query { .. }
        ) && displayed.selected_item.is_some()
        {
            header = header.child(
                ui::toolbar_button("save-item-snapshot", "Save snapshot").on_click(cx.listener(
                    |this, _, _, cx| {
                        let Some(displayed) = &this.state.reads.displayed else {
                            return;
                        };
                        let Some(item) = displayed
                            .selected_item
                            .and_then(|index| displayed.page.items.get(index))
                            .cloned()
                        else {
                            return;
                        };
                        let table = displayed.request.table.clone();
                        let connection = this.connection;
                        let Some(primary) = this.state.schema.schema().and_then(|schema| {
                            schema
                                .targets
                                .iter()
                                .find(|target| target.index_name.is_none())
                        }) else {
                            return;
                        };
                        this.history.update(cx, |history, cx| {
                            history.save_snapshot(connection, table, item, primary, cx)
                        });
                        this.notice =
                            Some("Snapshot queued for saving. Open History to view it.".into());
                        cx.notify();
                    },
                )),
            );
        }
        if page.next_key.is_some() && !self.state.reads.is_loading() {
            header = header.child(
                ui::toolbar_button(
                    "next-page",
                    if matches!(
                        displayed.request.operation,
                        crate::dynamodb::ReadOperation::Scan
                    ) {
                        "Prepare next scan page…"
                    } else {
                        "Next query page"
                    },
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(pending) = this.state.reads.next_page() {
                        this.submit(pending, window, cx);
                    }
                    cx.notify();
                })),
            );
        }
        let metrics = self.results_list.clone();
        let previous_width = self.results_width.clone();
        let items = div()
            .id("items-list")
            .test_support()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .border_r_1()
            .border_color(rgb(BORDER))
            // Keep spacing outside the virtual viewport so scrollbar and wheel
            // scrolling use the same content height.
            .p_3()
            .child(
                list(
                    self.results_list.clone(),
                    cx.processor(|this, index, _, cx| {
                        this.result_row(index, cx).into_any_element()
                    }),
                )
                .size_full(),
            )
            .child(
                canvas(
                    move |_, _, _| {
                        let width = metrics.viewport_bounds().size.width;
                        if previous_width.replace(width) != width {
                            // GPUI invalidates height hints when width changes. Reapply
                            // estimates after visible rows are laid out; retain their
                            // measured heights and avoid shaping every off-screen key.
                            metrics.clone().with_uniform_item_height(px(80.));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("items-scrollbar-overlay")
                    .test_support()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::vertical(&self.results_list).id("items-scrollbar")),
            );
        let detail = div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.detail.clone());
        div().flex().flex_col().flex_1().min_h_0().child(
            div()
                .id("results-pane")
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(header)
                .child(div().flex_1().min_h_0().child(SplitPane::new(
                    "result-panels",
                    &self.result_panels,
                    items,
                    detail,
                ))),
        )
    }
    fn result_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        self.rendered_rows.set(self.rendered_rows.get() + 1);
        let displayed = self
            .state
            .reads
            .displayed
            .as_ref()
            .expect("list belongs to a displayed page");
        let item = &displayed.page.items[index];
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
            .test_support()
            .w_full()
            .mb_2()
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
            .on_click(cx.listener(move |this, _, window, cx| {
                this.state.reads.select_item(index);
                this.sync_selected_item(window, cx);
                cx.notify();
            }))
    }
}
