use super::{
    ItemBrowser, PAGE_SIZE,
    query::{QueryDraft, ReadMode},
};
use crate::{
    dynamodb::SortOperator,
    ui::{self, ACCENT, BORDER, ERROR, HOVER, MUTED, PANEL, SELECTED},
};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{App, AppContext, Context, Div, Entity, Window, div, prelude::*, rgb};

pub(super) struct QueryInputs {
    pub target: usize,
    pub sort_operator: SortOperator,
    pub partition_input: Entity<InputState>,
    pub sort_input: Entity<InputState>,
    pub end_input: Entity<InputState>,
}

impl QueryInputs {
    pub fn new(window: &mut Window, cx: &mut Context<ItemBrowser>) -> Self {
        Self {
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

impl ItemBrowser {
    pub(super) fn controls(&self, cx: &mut Context<Self>) -> Div {
        // Don't render a partial form: its divider would jump from the collapsed
        // form's height to the full form's height when metadata arrives.
        if self.state.schema.is_loading() {
            return div().p_4().child(
                div()
                    .h_5()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .when(self.state.schema.loading_indicator_visible(), |status| {
                        status.child("Loading table keys…")
                    }),
            );
        }
        let mut controls = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(rgb(BORDER));
        controls = controls.child(div().flex().gap_2().children(ReadMode::ALL.into_iter().map(
            |mode| {
                ui::choice(mode.label(), mode.label(), self.mode == mode).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if this.mode != mode {
                            this.reset_read(cx);
                            this.mode = mode;
                        }
                        cx.notify();
                    },
                ))
            },
        )));
        if let Some(schema) = self.state.schema.schema() {
            controls = controls.child(div().flex().flex_wrap().gap_2().children(
                schema.targets.iter().enumerate().map(|(index, target)| {
                    div()
                        .id(("key-target", index))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(if self.inputs.target == index {
                            SELECTED
                        } else {
                            PANEL
                        }))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(HOVER)))
                        .child(target.label().to_owned())
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
            controls = controls.child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                "{} · up to {PAGE_SIZE} items per page",
                self.mode.description()
            )));
            if self.mode == ReadMode::Query {
                controls = controls
                    .child(div().text_sm().child(format!(
                        "Partition key: {} ({})",
                        target.partition.name,
                        target.partition.kind.as_str()
                    )))
                    .child(Input::new(&self.inputs.partition_input));
                if let Some(sort) = &target.sort {
                    controls = controls
                        .child(div().text_sm().child(format!(
                            "Sort key: {} ({})",
                            sort.name,
                            sort.kind.as_str()
                        )))
                        .child(
                            div().flex().flex_wrap().gap_2().children(
                                SortOperator::ALL
                                    .into_iter()
                                    .filter(|operator| {
                                        *operator != SortOperator::BeginsWith
                                            || sort.kind
                                                != aws_sdk_dynamodb::types::ScalarAttributeType::N
                                    })
                                    .map(|operator| {
                                        ui::choice(
                                            operator.label(),
                                            operator.label(),
                                            self.inputs.sort_operator == operator,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.reset_read(cx);
                                                this.inputs.sort_operator = operator;
                                                cx.notify();
                                            }),
                                        )
                                    }),
                            ),
                        );
                    if self.inputs.sort_operator != SortOperator::Any {
                        controls = controls.child(Input::new(&self.inputs.sort_input));
                    }
                    if self.inputs.sort_operator == SortOperator::Between {
                        controls = controls.child(Input::new(&self.inputs.end_input));
                    }
                }
                controls = controls.child(
                    ui::choice("run-query", "Run query", false)
                        .when(self.state.reads.is_loading(), |button| {
                            button.cursor_default().text_color(rgb(MUTED))
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.state.reads.is_loading() {
                                return;
                            }
                            match this.query_request(cx) {
                                Ok(request) => {
                                    if let Some(pending) = this.state.reads.query(request) {
                                        this.submit(pending, cx);
                                    }
                                }
                                Err(error) => {
                                    this.validation = Some(error);
                                    cx.notify();
                                }
                            }
                        })),
                );
            } else {
                controls = controls.child(
                    ui::choice("prepare-scan", "Prepare scan…", false)
                        .when(self.state.reads.is_loading(), |button| {
                            button.cursor_default().text_color(rgb(MUTED))
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.state.reads.is_loading() {
                                return;
                            }
                            if let Some(request) = this.scan_request() {
                                this.state.reads.prepare_scan(request, None);
                            }
                            cx.notify();
                        })),
                );
            }
        }
        if let Some(scan) = self.state.reads.prepared_scan() {
            controls = controls.child(div().flex().flex_col().gap_2().p_3().border_1().border_color(rgb(ACCENT)).rounded_md()
                .child(format!("Scan {} / {}?", scan.request.table, scan.request.target.label()))
                .child(div().text_sm().text_color(rgb(MUTED)).child("This reads one page of up to 50 items and can consume read capacity. Further pages require another confirmation."))
                .child(div().flex().gap_2()
                    .child(ui::choice("confirm-scan", "Confirm scan", true).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(pending) = this.state.reads.confirm_scan() { this.submit(pending, cx); }
                    })))
                    .child(ui::choice("cancel-scan", "Cancel", false).on_click(cx.listener(|this, _, _, cx| { this.state.reads.cancel_scan(); cx.notify(); })))));
        }
        // Reserve a status row so item reads don't shift the form's divider.
        controls = controls.child(div().h_5().text_sm().text_color(rgb(MUTED)).when(
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
                controls = controls.child(ui::choice("retry-schema", "Retry", false).on_click(
                    cx.listener(|this, _, window, cx| {
                        if let Some(table) = this.state.schema.table().map(str::to_owned) {
                            this.open(this.connection, table, window, cx);
                        }
                    }),
                ));
            }
        }
        if let Some(error) = &self.validation {
            controls = controls.child(div().text_color(rgb(ERROR)).child(error.clone()));
        }
        controls
    }
}
