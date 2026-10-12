mod item_detail;
mod query;
mod query_form;
mod results;
mod state;

use crate::{
    dynamodb::{Connection, DynamoDbService, KeyTarget, LoadError, ReadOperation, ReadRequest},
    ui,
};
use gpui_kit::{AppContext, Context, Entity, IntoElement, Render, Task, Window, div, prelude::*};
use item_detail::ItemDetail;
use query::ReadMode;
use query_form::QueryInputs;
use state::{BrowserState, PendingRead};
use std::time::Duration;

const PAGE_SIZE: i32 = 50;
const LOADING_INDICATOR_DELAY: Duration = Duration::from_millis(150);

pub struct ItemBrowser {
    service: DynamoDbService,
    connection: Connection,
    state: BrowserState,
    mode: ReadMode,
    inputs: QueryInputs,
    detail: Entity<ItemDetail>,
    validation: Option<String>,
    schema_task: Option<Task<()>>,
    schema_indicator_task: Option<Task<()>>,
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
            detail: cx.new(|cx| ItemDetail::new(window, cx)),
            validation: None,
            schema_task: None,
            schema_indicator_task: None,
            read_task: None,
        }
    }

    fn reset_read(&mut self, cx: &mut Context<Self>) {
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.state.reads.reset();
        self.validation = None;
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schema_task = None;
        self.schema_indicator_task = None;
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
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
        self.schema_indicator_task = None;
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.inputs.reset(window, cx);
        self.validation = None;
        self.connection = connection;
        let generation = self.state.open(table.clone());
        // Fast metadata responses never show a transient loading label.
        let timer = cx.background_executor().timer(LOADING_INDICATOR_DELAY);
        self.schema_indicator_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if this.state.reveal_schema_loading(generation) {
                    cx.notify();
                }
            });
        }));
        let request = self.service.describe_table(connection, table);
        self.schema_task = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.state.finish_schema(generation, result) {
                    this.schema_indicator_task = None;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(self.results(window, cx))
    }
}
