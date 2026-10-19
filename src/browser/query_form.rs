use super::{
    ItemBrowser, PAGE_SIZE,
    query::{QueryDraft, QueryField, ReadMode},
};
use crate::{
    dynamodb::SortOperator,
    ui::{self, ACCENT, BORDER, ERROR, MUTED},
};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    tooltip::Tooltip,
};
use gpui_kit::{
    App, AppContext, Context, Div, Entity, ScrollAnchor, ScrollHandle, Window, div, prelude::*, rgb,
};

pub(super) struct QueryInputs {
    pub partition_anchor: ScrollAnchor,
    pub sort_anchor: ScrollAnchor,
    pub end_anchor: ScrollAnchor,
    pub target: usize,
    pub sort_operator: SortOperator,
    pub partition_input: Entity<InputState>,
    pub sort_input: Entity<InputState>,
    pub end_input: Entity<InputState>,
}

impl QueryInputs {
    pub fn new(scroll: &ScrollHandle, window: &mut Window, cx: &mut Context<ItemBrowser>) -> Self {
        Self {
            partition_anchor: ScrollAnchor::for_handle(scroll.clone()),
            sort_anchor: ScrollAnchor::for_handle(scroll.clone()),
            end_anchor: ScrollAnchor::for_handle(scroll.clone()),
            target: 0,
            sort_operator: SortOperator::Any,
            partition_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Partition key value")),
            sort_input: cx.new(|cx| InputState::new(window, cx).placeholder("Sort key value")),
            end_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Range end (inclusive)")),
        }
    }

    pub fn clear_values(&mut self, window: &mut Window, cx: &mut Context<ItemBrowser>) {
        for input in [&self.partition_input, &self.sort_input, &self.end_input] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<ItemBrowser>) {
        self.target = 0;
        self.sort_operator = SortOperator::Any;
        self.clear_values(window, cx);
    }

    pub fn focus_field(
        &self,
        field: QueryField,
        window: &mut Window,
        cx: &mut Context<ItemBrowser>,
    ) {
        let (input, anchor) = match field {
            QueryField::Partition => (&self.partition_input, &self.partition_anchor),
            QueryField::Sort => (&self.sort_input, &self.sort_anchor),
            QueryField::End => (&self.end_input, &self.end_anchor),
        };
        anchor.scroll_to(window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn draft(&self, cx: &App) -> QueryDraft {
        QueryDraft {
            target: self.target,
            sort_operator: self.sort_operator,
            partition: self.partition_input.read(cx).value().to_string(),
            sort: self.sort_input.read(cx).value().to_string(),
            end: self.end_input.read(cx).value().to_string(),
        }
    }
}

fn query_value_input(
    id: &'static str,
    input: &Entity<InputState>,
    anchor: &ScrollAnchor,
) -> impl IntoElement {
    div()
        .id(format!("field:{id}"))
        .anchor_scroll(Some(anchor.clone()))
        .child(Input::new(input).id(id))
}

impl ItemBrowser {
    pub(super) fn query_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let summary = if self.mode == ReadMode::Query {
            self.query_request(cx)
                .map(|request| request.summary())
                .unwrap_or_else(|_| {
                    format!(
                        "Query · {} · enter key values",
                        self.key_target().map_or("Loading keys…", |t| t.label())
                    )
                })
        } else {
            format!(
                "Scan · {}",
                self.key_target().map_or("Loading keys…", |t| t.label())
            )
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                ui::toolbar_button(
                    "toggle-query-settings",
                    if self.settings_expanded {
                        "▾ Query settings"
                    } else {
                        "▸ Query settings"
                    },
                )
                .tooltip("Toggle query settings · Ctrl+Shift+Q")
                .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx))),
            )
            .child(
                div()
                    .id("query-draft-summary")
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .text_ellipsis()
                    .child(summary.clone())
                    .tooltip(move |window, cx| Tooltip::new(summary.clone()).build(window, cx)),
            )
            .child(
                ui::toolbar_button(
                    "run-query",
                    if self.mode == ReadMode::Query {
                        "Run query"
                    } else {
                        "Prepare scan…"
                    },
                )
                .disabled(self.state.reads.is_loading() || self.key_target().is_none())
                .tooltip(if self.mode == ReadMode::Query {
                    "Run query · Enter in a key field / Ctrl+Enter"
                } else {
                    "Prepare a scan for explicit confirmation"
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.mode == ReadMode::Query {
                        this.run_query(window, cx);
                    } else if !this.state.reads.is_loading() {
                        if let Some(request) = this.scan_request() {
                            this.state.reads.prepare_scan(request, None);
                        }
                        cx.notify();
                    }
                })),
            )
    }

    pub(super) fn controls(&self, cx: &mut Context<Self>) -> Div {
        let mut controls = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .border_b_1()
            .border_color(rgb(BORDER));
        if self.state.schema.is_loading() {
            return controls.when(self.state.schema.loading_indicator_visible(), |view| {
                view.child("Loading table keys…")
            });
        }
        controls = controls.child(div().flex().gap_2().children(ReadMode::ALL.into_iter().map(
            |mode| {
                ui::toolbar_button(mode.label(), mode.label())
                    .when(self.mode == mode, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.mode != mode {
                            this.reset_read(cx);
                            this.mode = mode;
                        }
                        cx.notify();
                    }))
            },
        )));
        if let Some(schema) = self.state.schema.schema() {
            controls = controls.child(div().flex().flex_wrap().gap_2().children(
                schema.targets.iter().enumerate().map(|(index, target)| {
                    Button::new(("key-target", index))
                        .small()
                        .outline()
                        .label(target.label().to_owned())
                        .when(self.inputs.target == index, |button| button.primary())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.inputs.target != index {
                                this.reset_read(cx);
                                this.inputs.target = index;
                                this.inputs.sort_operator = SortOperator::Any;
                                this.inputs.clear_values(window, cx);
                            }
                            cx.notify();
                        }))
                }),
            ));
        }
        if let Some(target) = self.key_target() {
            if self.mode == ReadMode::Query {
                let mut fields = div().flex().flex_wrap().gap_3().child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(gpui_kit::px(180.))
                        .gap_2()
                        .child(div().text_sm().child(format!(
                            "{} ({})",
                            target.partition.name,
                            target.partition.kind.as_str()
                        )))
                        .child(query_value_input(
                            "query-partition-input",
                            &self.inputs.partition_input,
                            &self.inputs.partition_anchor,
                        )),
                );
                if let Some(sort) = &target.sort {
                    let mut sort_fields = div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(gpui_kit::px(180.))
                        .gap_2()
                        .child(div().text_sm().child(format!(
                            "{} ({})",
                            sort.name,
                            sort.kind.as_str()
                        )))
                        .child(
                            div().flex().flex_wrap().gap_1().children(
                                SortOperator::ALL
                                    .into_iter()
                                    .filter(|operator| {
                                        *operator != SortOperator::BeginsWith
                                            || sort.kind
                                                != aws_sdk_dynamodb::types::ScalarAttributeType::N
                                    })
                                    .map(|operator| {
                                        ui::toolbar_button(operator.label(), operator.label())
                                            .when(self.inputs.sort_operator == operator, |button| {
                                                button.primary()
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.reset_read(cx);
                                                this.inputs.sort_operator = operator;
                                                cx.notify();
                                            }))
                                    }),
                            ),
                        );
                    if self.inputs.sort_operator != SortOperator::Any {
                        sort_fields = sort_fields.child(query_value_input(
                            "query-sort-input",
                            &self.inputs.sort_input,
                            &self.inputs.sort_anchor,
                        ));
                    }
                    if self.inputs.sort_operator == SortOperator::Between {
                        sort_fields = sort_fields.child(query_value_input(
                            "query-end-input",
                            &self.inputs.end_input,
                            &self.inputs.end_anchor,
                        ));
                    }
                    fields = fields.child(sort_fields);
                }
                controls = controls.child(fields);
            }
            controls = controls.child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                "{} · up to {PAGE_SIZE} items per page",
                self.mode.description()
            )));
        }
        controls
    }

    pub(super) fn status_controls(&self, cx: &mut Context<Self>) -> Div {
        if self.state.reads.prepared_scan().is_none()
            && !self.state.reads.is_loading()
            && self.error().is_none()
            && self.validation.is_none()
            && self.notice.is_none()
        {
            return div();
        }
        let mut controls = div().flex().flex_col().gap_2().p_3();
        if let Some(scan) = self.state.reads.prepared_scan() {
            controls = controls.child(div().flex().flex_col().gap_2().p_3().border_1().border_color(rgb(ACCENT)).rounded_md()
                .child(format!("Scan {} / {}?", scan.request.table, scan.request.target.label()))
                .child(div().text_sm().text_color(rgb(MUTED)).child("This reads one page of up to 50 items and can consume read capacity. Further pages require another confirmation."))
                .child(div().flex().gap_2()
                    .child(ui::toolbar_button("confirm-scan", "Confirm scan").primary().on_click(cx.listener(|this, _, window, cx| {
                        if let Some(pending) = this.state.reads.confirm_scan() { this.submit(pending, window, cx); }
                    })))
                    .child(ui::toolbar_button("cancel-scan", "Cancel").on_click(cx.listener(|this, _, _, cx| { this.state.reads.cancel_scan(); cx.notify(); })))));
        }
        controls = controls.child(div().text_sm().text_color(rgb(MUTED)).when(
            self.state.reads.is_loading(),
            |status| {
                status.child(if self.state.reads.displayed.is_some() {
                    "Reading one page… Previous results remain visible."
                } else {
                    "Reading one page…"
                })
            },
        ));
        if let Some(error) = self.error() {
            controls = controls
                .child(div().text_color(rgb(ERROR)).child(error.title()))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(error.message(self.connection)),
                );
            if self.state.schema.schema().is_none()
                && self.state.schema.table().is_some()
                && !self.state.schema.is_loading()
            {
                controls = controls.child(ui::toolbar_button("retry-schema", "Retry").on_click(
                    cx.listener(|this, _, window, cx| {
                        if let Some(table) = this.state.schema.table().map(str::to_owned) {
                            let restore = this.restore.clone();
                            this.open(this.connection, table, window, cx);
                            this.restore = restore;
                        }
                    }),
                ));
            }
        }
        if let Some(error) = &self.validation {
            controls = controls.child(div().text_color(rgb(ERROR)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            controls = controls.child(div().text_sm().text_color(rgb(MUTED)).child(notice.clone()));
        }
        controls
    }
}
