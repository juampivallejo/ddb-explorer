use gpui::{Context, Div, IntoElement, Render, Task, Window, div, prelude::*, rgb};

use crate::{
    dynamodb::{self, Connection, ReadMode},
    ui::{self, ACCENT, BACKGROUND, BORDER, MUTED, PANEL, TEXT},
};

#[derive(Default)]
pub struct Explorer {
    connection: Connection,
    read_mode: ReadMode,
    tables: Vec<String>,
    selected_table: Option<String>,
    loading: bool,
    error: Option<String>,
    request_id: u64,
    load_task: Option<Task<()>>,
}

impl Explorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut explorer = Self::default();
        explorer.refresh_tables(cx);
        explorer
    }

    fn refresh_tables(&mut self, cx: &mut Context<Self>) {
        self.request_id += 1;
        let request_id = self.request_id;
        let connection = self.connection;
        self.loading = true;
        self.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { dynamodb::load_tables(connection) });
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
                    cx.listener(move |this, _, _, cx| {
                        this.tables.clear();
                        this.selected_table = None;
                        this.connection = connection;
                        this.refresh_tables(cx);
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

    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .w_80()
            .h_full()
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(self.connection_panel(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(format!("TABLES ({})", self.tables.len())),
                            )
                            .child(
                                ui::choice("refresh-tables", "Refresh", false).on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_tables(cx)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id("table-list")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .gap_2()
                            .text_sm()
                            .children(self.tables.iter().enumerate().map(|(index, table)| {
                                let selected = self.selected_table.as_ref() == Some(table);
                                let name = table.clone();
                                div()
                                    .id(("table", index))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(if selected { 0x25344e } else { PANEL }))
                                    .text_color(rgb(if selected { ACCENT } else { TEXT }))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x2a3445)))
                                    .child(table.clone())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected_table = Some(name.clone());
                                        cx.notify();
                                    }))
                            }))
                            .when(self.loading, |list| list.child("Loading tables…"))
                            .when(
                                !self.loading && self.error.is_none() && self.tables.is_empty(),
                                |list| list.child("No tables in this connection"),
                            )
                            .when_some(self.error.as_ref(), |list, error| {
                                list.child(div().text_color(rgb(0xffa3a3)).child(error.clone()))
                            }),
                    ),
            )
    }

    fn workspace(&self, cx: &mut Context<Self>) -> Div {
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_6()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children(ReadMode::ALL.into_iter().map(|mode| {
                                let id = match mode {
                                    ReadMode::Scan => "scan-mode",
                                    ReadMode::Query => "query-mode",
                                };
                                ui::choice(id, mode.label(), self.read_mode == mode).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.read_mode = mode;
                                        cx.notify();
                                    }),
                                )
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.read_mode.description()),
                    ),
            )
            .child(div().flex().flex_col().flex_1().justify_center().child(
                if self.selected_table.is_some() {
                    ui::empty_state("Table selected", "Item browsing is not available yet.")
                } else {
                    ui::empty_state(
                        "Select a table to explore",
                        "Choose a table from the sidebar.",
                    )
                },
            ))
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
                    .child(div().text_lg().child("DynamoDB Explorer"))
                    .child(div().text_sm().text_color(rgb(ACCENT)).child("Read only")),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(self.workspace(cx)),
            )
    }
}
