use gpui_kit::{
    ClipboardItem, Context, Div, Entity, IntoElement, Render, Task, Window, div, prelude::*, rgb,
};

use gpui_kit::component::input::{Input, InputState};

use crate::{
    dynamodb::{
        self, Connection, ItemPage, KeyTarget, LoadError, ReadMode, ReadOperation, ReadRequest,
        SortOperator, TableSchema,
    },
    ui::{self, ACCENT, BORDER, ERROR, HOVER, MUTED, PANEL, SELECTED},
};

const PAGE_SIZE: i32 = 50;

struct PreparedScan {
    request: ReadRequest,
    start: Option<dynamodb::Item>,
}

pub struct ItemBrowser {
    connection: Connection,
    table: Option<String>,
    schema: Option<TableSchema>,
    target: usize,
    mode: ReadMode,
    sort_operator: SortOperator,
    partition_input: Entity<InputState>,
    sort_input: Entity<InputState>,
    end_input: Entity<InputState>,
    schema_loading: bool,
    read_loading: bool,
    error: Option<LoadError>,
    validation: Option<String>,
    prepared_scan: Option<PreparedScan>,
    last_request: Option<ReadRequest>,
    page: Option<ItemPage>,
    selected_item: Option<usize>,
    copied: bool,
    generation: u64,
    read_generation: u64,
    schema_task: Option<Task<()>>,
    read_task: Option<Task<()>>,
}

impl ItemBrowser {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            connection: Connection::Local,
            table: None,
            schema: None,
            target: 0,
            mode: ReadMode::Query,
            sort_operator: SortOperator::Any,
            partition_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Partition key value")),
            sort_input: cx.new(|cx| InputState::new(window, cx).placeholder("Sort key value")),
            end_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Range end (inclusive)")),
            schema_loading: false,
            read_loading: false,
            error: None,
            validation: None,
            prepared_scan: None,
            last_request: None,
            page: None,
            selected_item: None,
            copied: false,
            generation: 0,
            read_generation: 0,
            schema_task: None,
            read_task: None,
        }
    }

    fn reset_read(&mut self, cx: &mut Context<Self>) {
        self.read_generation += 1;
        self.read_task = None;
        self.read_loading = false;
        self.prepared_scan = None;
        self.last_request = None;
        self.page = None;
        self.selected_item = None;
        self.copied = false;
        self.error = None;
        self.validation = None;
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.schema_task = None;
        self.schema_loading = false;
        self.schema = None;
        self.table = None;
        self.target = 0;
        self.sort_operator = SortOperator::Any;
        self.partition_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.sort_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.end_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reset_read(cx);
    }

    pub fn open(
        &mut self,
        connection: Connection,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear(window, cx);
        self.connection = connection;
        self.table = Some(table.clone());
        self.schema_loading = true;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { dynamodb::describe_table(connection, table) });
        self.schema_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.schema_loading = false;
                match result {
                    Ok(schema) => this.schema = Some(schema),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn key_target(&self) -> Option<&KeyTarget> {
        self.schema.as_ref()?.targets.get(self.target)
    }

    fn scan_request(&self) -> Option<ReadRequest> {
        Some(ReadRequest {
            table: self.table.clone()?,
            target: self.key_target()?.clone(),
            operation: ReadOperation::Scan,
            limit: PAGE_SIZE,
        })
    }

    fn query_request(&self, cx: &Context<Self>) -> Result<ReadRequest, String> {
        let target = self.key_target().ok_or("Select a table first.")?;
        let partition = target
            .partition
            .parse(self.partition_input.read(cx).value().as_ref())?;
        let sort = if self.sort_operator == SortOperator::Any {
            None
        } else {
            let key = target.sort.as_ref().ok_or("This key has no sort key.")?;
            if self.sort_operator == SortOperator::BeginsWith
                && key.kind == aws_sdk_dynamodb::types::ScalarAttributeType::N
            {
                return Err(
                    "Begins with is available for string and binary sort keys only.".into(),
                );
            }
            let value = key.parse(self.sort_input.read(cx).value().as_ref())?;
            let end = if self.sort_operator == SortOperator::Between {
                Some(key.parse(self.end_input.read(cx).value().as_ref())?)
            } else {
                None
            };
            Some((self.sort_operator, value, end))
        };
        Ok(ReadRequest {
            table: self.table.clone().ok_or("Select a table first.")?,
            target: target.clone(),
            operation: ReadOperation::Query { partition, sort },
            limit: PAGE_SIZE,
        })
    }

    fn execute(
        &mut self,
        request: ReadRequest,
        start: Option<dynamodb::Item>,
        cx: &mut Context<Self>,
    ) {
        if self.read_loading {
            return;
        }
        self.read_generation += 1;
        let generation = self.read_generation;
        self.error = None;
        self.validation = None;
        self.prepared_scan = None;
        self.read_loading = true;
        self.page = None;
        self.selected_item = None;
        self.copied = false;
        self.last_request = Some(request.clone());
        let connection = self.connection;
        let task = cx
            .background_executor()
            .spawn(async move { dynamodb::read_items(connection, request, start) });
        self.read_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.read_generation != generation {
                    return;
                }
                this.read_loading = false;
                match result {
                    Ok(page) => {
                        this.selected_item = (!page.items.is_empty()).then_some(0);
                        this.page = Some(page);
                    }
                    Err(error) => {
                        this.page = None;
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn controls(&self, cx: &mut Context<Self>) -> Div {
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
        if let Some(schema) = &self.schema {
            controls = controls.child(div().flex().flex_wrap().gap_2().children(
                schema.targets.iter().enumerate().map(|(index, target)| {
                    div()
                        .id(("key-target", index))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(if self.target == index {
                            SELECTED
                        } else {
                            PANEL
                        }))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(HOVER)))
                        .child(target.label().to_owned())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.target != index {
                                this.reset_read(cx);
                                this.target = index;
                                this.sort_operator = SortOperator::Any;
                                this.partition_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.sort_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.end_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
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
                    .child(Input::new(&self.partition_input));
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
                                            self.sort_operator == operator,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.reset_read(cx);
                                                this.sort_operator = operator;
                                                cx.notify();
                                            }),
                                        )
                                    }),
                            ),
                        );
                    if self.sort_operator != SortOperator::Any {
                        controls = controls.child(Input::new(&self.sort_input));
                    }
                    if self.sort_operator == SortOperator::Between {
                        controls = controls.child(Input::new(&self.end_input));
                    }
                }
                if !self.read_loading {
                    controls =
                        controls.child(ui::choice("run-query", "Run query", false).on_click(
                            cx.listener(|this, _, _, cx| match this.query_request(cx) {
                                Ok(request) => this.execute(request, None, cx),
                                Err(error) => {
                                    this.validation = Some(error);
                                    cx.notify();
                                }
                            }),
                        ));
                }
            } else if !self.read_loading {
                controls =
                    controls.child(ui::choice("prepare-scan", "Prepare scan…", false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.prepared_scan = this.scan_request().map(|request| PreparedScan {
                                request,
                                start: None,
                            });
                            cx.notify();
                        }),
                    ));
            }
        }
        if let Some(scan) = &self.prepared_scan {
            controls = controls.child(div().flex().flex_col().gap_2().p_3().border_1().border_color(rgb(ACCENT)).rounded_md()
                .child(format!("Scan {} / {}?", scan.request.table, scan.request.target.label()))
                .child(div().text_sm().text_color(rgb(MUTED)).child("This reads one page of up to 50 items and can consume read capacity. Further pages require another confirmation."))
                .child(div().flex().gap_2()
                    .child(ui::choice("confirm-scan", "Confirm scan", true).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(scan) = this.prepared_scan.take() { this.execute(scan.request, scan.start, cx); }
                    })))
                    .child(ui::choice("cancel-scan", "Cancel", false).on_click(cx.listener(|this, _, _, cx| { this.prepared_scan = None; cx.notify(); })))));
        }
        if self.schema_loading {
            controls = controls.child("Loading table keys…");
        }
        if self.read_loading {
            controls = controls.child("Reading one page…");
        }
        if let Some(error) = self.error {
            controls = controls
                .child(div().text_color(rgb(ERROR)).child(error.title()))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(error.message(self.connection)),
                );
            if self.schema.is_none() && self.table.is_some() && !self.schema_loading {
                controls = controls.child(ui::choice("retry-schema", "Retry", false).on_click(
                    cx.listener(|this, _, window, cx| {
                        if let Some(table) = this.table.clone() {
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

    fn results(&self, cx: &mut Context<Self>) -> Div {
        let Some(page) = &self.page else {
            return div().flex_1().child(ui::empty_state(
                "No items loaded",
                "Run a query or prepare a scan to read a page.",
            ));
        };
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .p_3()
            .child(div().flex_1().min_w_0().text_sm().child(format!(
                    "{} items in this page · {}",
                    page.items.len(),
                    self.last_request
                        .as_ref()
                        .map(ReadRequest::summary)
                        .unwrap_or_default()
                )));
        if page.next_key.is_some() && !self.read_loading {
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
                    if let Some(request) = this.last_request.clone() {
                        let start = this.page.as_ref().and_then(|page| page.next_key.clone());
                        if start.is_none() {
                            return;
                        }
                        if matches!(request.operation, ReadOperation::Scan) {
                            this.prepared_scan = Some(PreparedScan { request, start });
                            cx.notify();
                        } else {
                            this.execute(request, start, cx);
                        }
                    }
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
                let target = self.last_request.as_ref().map(|request| &request.target);
                let title = target
                    .and_then(|target| item.get(&target.partition.name))
                    .map(serde_json::Value::to_string)
                    .unwrap_or_else(|| format!("Item {}", index + 1));
                let sort = target
                    .and_then(|target| target.sort.as_ref())
                    .and_then(|key| item.get(&key.name))
                    .map(serde_json::Value::to_string);
                div()
                    .id(("item", index))
                    .p_2()
                    .rounded_md()
                    .bg(rgb(if self.selected_item == Some(index) {
                        SELECTED
                    } else {
                        PANEL
                    }))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(title)
                    .when_some(sort, |item, sort| {
                        item.child(div().text_sm().text_color(rgb(MUTED)).child(sort))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_item = Some(index);
                        this.copied = false;
                        cx.notify();
                    }))
            }));
        let mut detail = div().flex().flex_col().flex_1().min_w_0().min_h_0();
        if let Some(item) = self.selected_item.and_then(|index| page.items.get(index)) {
            let json = serde_json::to_string_pretty(item).expect("JSON values serialize");
            let copy = json.clone();
            detail = detail
                .child(
                    div().p_3().child(
                        ui::choice(
                            "copy-json",
                            if self.copied { "Copied" } else { "Copy JSON" },
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                            this.copied = true;
                            cx.notify();
                        })),
                    ),
                )
                .child(
                    div()
                        .id("json-detail")
                        .flex_1()
                        .min_h_0()
                        .overflow_scroll()
                        .p_4()
                        .font_family("monospace")
                        .text_sm()
                        .children(json.lines().map(|line| div().child(line.to_owned()))),
                );
        } else {
            detail = detail.child(ui::empty_state(
                "No matching items",
                "Try another key value or read the next page if available.",
            ));
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(div().flex().flex_1().min_h_0().child(list).child(detail))
    }
}

impl Render for ItemBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.table.is_none() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .justify_center()
                .child(ui::empty_state(
                    "Select a table to explore",
                    "Choose a table from the sidebar.",
                ));
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("read-controls")
                    .max_h(gpui_kit::relative(0.6))
                    .overflow_y_scroll()
                    .child(self.controls(cx)),
            )
            .child(self.results(cx))
    }
}
