mod query;
mod query_form;
mod results;
mod state;

use crate::{
    dynamodb::{Connection, DynamoDbService, KeyTarget, LoadError, ReadOperation, ReadRequest},
    ui,
};
use gpui_kit::{Context, IntoElement, Render, Task, Window, div, prelude::*};
use query::ReadMode;
use query_form::QueryInputs;
use state::{BrowserState, PendingRead};

const PAGE_SIZE: i32 = 50;

pub struct ItemBrowser {
    service: DynamoDbService,
    connection: Connection,
    state: BrowserState,
    mode: ReadMode,
    inputs: QueryInputs,
    validation: Option<String>,
    schema_task: Option<Task<()>>,
    read_task: Option<Task<()>>,
}

impl ItemBrowser {
    pub fn new(service: DynamoDbService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            service,
            connection: Connection::Local,
            state: BrowserState::default(),
            mode: ReadMode::default(),
            inputs: QueryInputs::new(window, cx),
            validation: None,
            schema_task: None,
            read_task: None,
        }
    }

    fn reset_read(&mut self, cx: &mut Context<Self>) {
        self.read_task = None;
        self.state.reads.reset();
        self.validation = None;
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schema_task = None;
        self.read_task = None;
        self.state.clear();
        self.inputs.reset(window, cx);
        self.validation = None;
        cx.notify();
    }

    pub fn open(
        &mut self,
        connection: Connection,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.schema_task = None;
        self.read_task = None;
        self.inputs.reset(window, cx);
        self.validation = None;
        self.connection = connection;
        let generation = self.state.open(table.clone());
        let request = self.service.describe_table(connection, table);
        self.schema_task = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.state.finish_schema(generation, result) {
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn key_target(&self) -> Option<&KeyTarget> {
        self.state.schema.schema()?.targets.get(self.inputs.target)
    }

    fn error(&self) -> Option<LoadError> {
        self.state
            .schema
            .error()
            .or_else(|| self.state.reads.error())
    }

    fn scan_request(&self) -> Option<ReadRequest> {
        Some(ReadRequest {
            table: self.state.schema.table()?.to_owned(),
            target: self.key_target()?.clone(),
            operation: ReadOperation::Scan,
            limit: PAGE_SIZE,
        })
    }

    fn query_request(&self, cx: &Context<Self>) -> Result<ReadRequest, String> {
        let table = self.state.schema.table().ok_or("Select a table first.")?;
        let schema = self
            .state
            .schema
            .schema()
            .ok_or("Wait for the table keys to load.")?;
        self.inputs.draft(cx).validate(table, schema, PAGE_SIZE)
    }

    fn submit(&mut self, pending: PendingRead, cx: &mut Context<Self>) {
        self.validation = None;
        let id = pending.id;
        let request = self
            .service
            .read_items(self.connection, pending.request, pending.start);
        self.read_task = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.state.reads.finish(id, result) {
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}

impl Render for ItemBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.schema.table().is_none() {
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
