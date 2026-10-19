pub(crate) mod item_detail;
mod query;
mod query_form;
mod results;
mod state;

use crate::ui::split::SplitState;
use crate::{
    dynamodb::{Connection, DynamoDbService, KeyTarget, LoadError, ReadOperation, ReadRequest},
    history::{Entry, History, SavedQuery},
    ui,
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ListAlignment, ListState, Render, Task, Window, div,
    prelude::*,
};
use item_detail::ItemDetail;
use query::ReadMode;
use query_form::QueryInputs;
use state::{BrowserState, PendingRead};
use std::{cell::Cell, rc::Rc, time::Duration};

const PAGE_SIZE: i32 = 50;
const LOADING_INDICATOR_DELAY: Duration = Duration::from_millis(150);

pub struct ItemBrowser {
    results_list: ListState,
    results_width: Rc<Cell<gpui_kit::Pixels>>,
    #[cfg(test)]
    rendered_rows: Cell<usize>,
    result_panels: Entity<SplitState>,
    service: DynamoDbService,
    connection: Connection,
    state: BrowserState,
    mode: ReadMode,
    inputs: QueryInputs,
    detail: Entity<ItemDetail>,
    validation: Option<String>,
    notice: Option<String>,
    schema_task: Option<Task<()>>,
    schema_indicator_task: Option<Task<()>>,
    read_task: Option<Task<()>>,
    history: Entity<History>,
    active_history: Option<Entry>,
    restore: Option<SavedQuery>,
}

impl ItemBrowser {
    pub fn new(
        service: DynamoDbService,
        history: Entity<History>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            results_list: ListState::new(0, ListAlignment::Top, gpui_kit::px(100.))
                .with_uniform_item_height(gpui_kit::px(80.)),
            results_width: Rc::new(Cell::new(gpui_kit::px(0.))),
            #[cfg(test)]
            rendered_rows: Cell::new(0),
            result_panels: cx.new(|_| {
                SplitState::new(gpui_kit::px(280.), gpui_kit::px(160.), gpui_kit::px(160.))
            }),
            service,
            connection: Connection::Local,
            state: BrowserState::default(),
            mode: ReadMode::default(),
            inputs: QueryInputs::new(window, cx),
            detail: cx.new(|cx| ItemDetail::new(window, cx)),
            validation: None,
            notice: None,
            schema_task: None,
            schema_indicator_task: None,
            read_task: None,
            history,
            active_history: None,
            restore: None,
        }
    }

    fn reset_read(&mut self, cx: &mut Context<Self>) {
        self.active_history = None;
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.state.reads.reset();
        self.results_list.reset(0);
        self.validation = None;
        self.notice = None;
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_history = None;
        self.restore = None;
        self.schema_task = None;
        self.schema_indicator_task = None;
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.state.clear();
        self.results_list.reset(0);
        self.inputs.reset(window, cx);
        self.validation = None;
        self.notice = None;
        cx.notify();
    }

    pub fn open(
        &mut self,
        connection: Connection,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_history = None;
        self.restore = None;
        self.schema_task = None;
        self.schema_indicator_task = None;
        self.read_task = None;
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.inputs.reset(window, cx);
        self.validation = None;
        self.notice = None;
        self.connection = connection;
        let generation = self.state.open(table.clone());
        self.results_list.reset(0);
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
        self.schema_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.state.finish_schema(generation, result) {
                    this.schema_indicator_task = None;
                    if this.state.schema.schema().is_some()
                        && let Some(query) = this.restore.take()
                    {
                        this.apply_restore(query, window, cx);
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub fn restore_query(
        &mut self,
        connection: Connection,
        table: String,
        query: SavedQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(connection, table, window, cx);
        self.mode = ReadMode::Query;
        self.restore = Some(query);
    }

    fn apply_restore(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .state
            .schema
            .schema()
            .and_then(|schema| query.target_index(schema));
        let Some(target) = target else {
            self.validation = Some(
                "The saved query's key or index is unavailable on the current connection.".into(),
            );
            return;
        };
        self.inputs.target = target;
        self.inputs.sort_operator = query.operator;
        for (input, value) in [
            (&self.inputs.partition_input, query.partition),
            (&self.inputs.sort_input, query.sort),
            (&self.inputs.end_input, query.end),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.notice = Some(format!(
            "Restored on {}. Review the values, then run the query.",
            self.connection.label()
        ));
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

    fn submit(&mut self, pending: PendingRead, window: &mut Window, cx: &mut Context<Self>) {
        self.validation = None;
        self.notice = None;
        let id = pending.id;
        let first_page = pending.start.is_none();
        let request = self
            .service
            .read_items(self.connection, pending.request, pending.start);
        self.read_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.state.reads.finish(id, result) {
                    if this.state.reads.error().is_none() {
                        this.sync_results(window, cx);
                        this.record_history(first_page, cx);
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn sync_results(&self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self
            .state
            .reads
            .displayed
            .as_ref()
            .map_or(0, |page| page.page.items.len());
        self.results_list
            .reset_with_uniform_height(count, gpui_kit::px(80.));
        self.sync_selected_item(window, cx);
    }

    fn sync_selected_item(&self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.state.reads.displayed.as_ref().and_then(|page| {
            page.selected_item.and_then(|index| {
                page.page
                    .items
                    .get(index)
                    .map(|item| ((page.id, index), item))
            })
        });
        self.detail
            .update(cx, |detail, cx| detail.set_item(selected, window, cx));
    }

    fn record_history(&mut self, first_page: bool, cx: &mut Context<Self>) {
        let Some(displayed) = &self.state.reads.displayed else {
            return;
        };
        let Some(query) = SavedQuery::from_request(&displayed.request) else {
            return;
        };
        if first_page {
            self.active_history = Some(Entry::new(
                self.connection,
                displayed.request.table.clone(),
                Some(query),
            ));
        }
        let Some(entry) = &mut self.active_history else {
            return;
        };
        let Some(primary) = self.state.schema.schema().and_then(|schema| {
            schema
                .targets
                .iter()
                .find(|target| target.index_name.is_none())
        }) else {
            return;
        };
        entry.add_page(&displayed.page, primary);
        let entry = entry.clone();
        self.history
            .update(cx, |history, cx| history.record(entry, first_page, cx));
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
#[cfg(test)]
mod tests;
