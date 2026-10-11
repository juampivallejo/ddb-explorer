use gpui::{Context, Div, IntoElement, Render, Window, div, prelude::*, rgb};

use crate::{
    dynamodb::{Connection, ReadMode},
    ui::{self, ACCENT, BACKGROUND, BORDER, MUTED, PANEL, TEXT},
};

#[derive(Default)]
pub struct Explorer {
    connection: Connection,
    read_mode: ReadMode,
}

impl Explorer {
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
                        this.connection = connection;
                        cx.notify();
                    }),
                )
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Not connected"),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .w_64()
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
                    .p_4()
                    .gap_4()
                    .child(div().text_sm().text_color(rgb(MUTED)).child("TABLES"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("No tables loaded"),
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
                    .child(div().text_xl().child(self.connection.label()))
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .justify_center()
                    .child(ui::empty_state(
                        "Select a table to explore",
                        "Items and table details will appear here.",
                    )),
            )
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
