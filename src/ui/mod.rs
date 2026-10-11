//! Small shared view components and colors.

use gpui::{Div, Stateful, div, prelude::*, rgb};

pub const BACKGROUND: u32 = 0x111318;
pub const PANEL: u32 = 0x191c23;
pub const BORDER: u32 = 0x2c313c;
pub const TEXT: u32 = 0xe6e9ef;
pub const MUTED: u32 = 0x9aa4b5;
pub const ACCENT: u32 = 0x95b8ff;
pub const ERROR: u32 = 0xffa3a3;

pub fn choice(id: &'static str, label: &'static str, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(if selected { ACCENT } else { BORDER }))
        .bg(rgb(if selected { 0x25344e } else { PANEL }))
        .text_color(rgb(if selected { ACCENT } else { TEXT }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x2a3445)))
        .child(label)
}

pub fn empty_state(title: &'static str, message: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(div().text_lg().child(title))
        .child(div().text_sm().text_color(rgb(MUTED)).child(message))
}
