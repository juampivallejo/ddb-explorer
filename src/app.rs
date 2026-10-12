mod table_sidebar;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext, Context, Div, Entity, IntoElement, Render, Subscription, Task,
    UniformListScrollHandle, Window, div, prelude::*, rgb,
};
use table_sidebar::VisibleTable;

use crate::{
    browser::ItemBrowser,
    dynamodb::{Connection, DynamoDbService, LoadError},
    history::{History, RestoreQuery},
    preferences::{FavoriteChange, PreferenceError, PreferenceStore, Preferences},
    ui::{self, ACCENT, BACKGROUND, BORDER, MUTED, TEXT},
};

pub struct Explorer {
    service: DynamoDbService,
    connection: Connection,
    browser: Entity<ItemBrowser>,
    tables: Vec<String>,
    selected_table: Option<String>,
    loading: bool,
    error: Option<LoadError>,
    request_id: u64,
    load_task: Option<Task<()>>,
    table_filter: Entity<InputState>,
    filter_text: String,
    favorites_only: bool,
    visible_tables: Vec<VisibleTable>,
    table_scroll: UniformListScrollHandle,
    preferences: Preferences,
    preference_store: Option<PreferenceStore>,
    preference_error: Option<PreferenceError>,
    save_id: u64,
    save_task: Option<Task<()>>,
    _filter_subscription: Subscription,
    history: Entity<History>,
    history_visible: bool,
    _history_subscription: Subscription,
    _history_observer: Subscription,
}

impl Explorer {
    pub fn new(service: DynamoDbService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let history = cx.new(|cx| History::new(window, cx));
        let history_observer = cx.observe(&history, |_, _, cx| cx.notify());
        let history_subscription = cx.subscribe_in(
            &history,
            window,
            |this, _, event: &RestoreQuery, window, cx| {
                this.history_visible = false;
                this.selected_table = Some(event.table.clone());
                this.browser.update(cx, |browser, cx| {
                    browser.restore_query(
                        this.connection,
                        event.table.clone(),
                        event.query.clone(),
                        window,
                        cx,
                    )
                });
                cx.notify();
            },
        );
        let table_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter tables…"));
        let filter_subscription = cx.subscribe(&table_filter, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter_text = input.read(cx).value().to_string();
                this.reset_table_list();
                cx.notify();
            }
        });
        let (preference_store, preferences, preference_error) = match PreferenceStore::open() {
            Ok((store, preferences)) => (Some(store), preferences, None),
            Err(error) => (None, Preferences::default(), Some(error)),
        };
        let mut explorer = Self {
            connection: Connection::Local,
            browser: cx.new(|cx| ItemBrowser::new(service.clone(), history.clone(), window, cx)),
            service,
            tables: Vec::new(),
            selected_table: None,
            loading: false,
            error: None,
            request_id: 0,
            load_task: None,
            table_filter,
            filter_text: String::new(),
            favorites_only: false,
            visible_tables: Vec::new(),
            table_scroll: UniformListScrollHandle::new(),
            preferences,
            preference_store,
            preference_error,
            save_id: 0,
            save_task: None,
            _filter_subscription: filter_subscription,
            history,
            history_visible: false,
            _history_subscription: history_subscription,
            _history_observer: history_observer,
        };
        explorer.refresh_tables(window, cx);
        explorer
    }

    fn refresh_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_table = None;
        self.browser
            .update(cx, |browser, cx| browser.clear(window, cx));
        self.request_id += 1;
        let request_id = self.request_id;
        let connection = self.connection;
        self.loading = true;
        self.error = None;
        let task = self.service.list_tables(connection);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                // A slow response must not replace a newer connection's tables.
                if this.request_id != request_id {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(tables) => {
                        if this
                            .selected_table
                            .as_ref()
                            .is_some_and(|table| !tables.contains(table))
                        {
                            this.selected_table = None;
                        }
                        this.tables = tables;
                    }
                    Err(error) => {
                        this.tables.clear();
                        this.selected_table = None;
                        this.error = Some(error);
                    }
                }
                this.reset_table_list();
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn connection_panel(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().text_sm().text_color(rgb(MUTED)).child("CONNECTION"))
            .children(Connection::ALL.into_iter().map(|connection| {
                let id = match connection {
                    Connection::Local => "local-connection",
                    Connection::Terminal => "terminal-connection",
                };
                ui::choice(id, connection.label(), self.connection == connection).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.tables.clear();
                        this.selected_table = None;
                        this.connection = connection;
                        this.reset_table_list();
                        this.refresh_tables(window, cx);
                    }),
                )
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(if self.loading {
                        "Loading tables…"
                    } else if self.error.is_some() {
                        "Connection failed"
                    } else {
                        "Connected"
                    }),
            )
    }

    fn workspace(&self, _cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_6()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div().text_xl().child(
                            self.selected_table
                                .clone()
                                .unwrap_or_else(|| self.connection.label().to_owned()),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.connection.description()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.browser.clone()))
    }
}

impl Render for Explorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_lg().child("DynamoDB Explorer"))
                            .child(
                                ui::choice("explorer-tab", "Explorer", !self.history_visible)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.history_visible = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                ui::choice(
                                    "history-tab",
                                    if self.history.read(cx).has_error() {
                                        "History · storage error"
                                    } else {
                                        "History"
                                    },
                                    self.history_visible,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.history_visible = true;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(div().text_sm().text_color(rgb(ACCENT)).child("Read only")),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(!self.history_visible, |view| {
                        view.child(self.sidebar(cx)).child(self.workspace(cx))
                    })
                    .when(self.history_visible, |view| {
                        view.child(self.history.clone())
                    }),
            )
    }
}
