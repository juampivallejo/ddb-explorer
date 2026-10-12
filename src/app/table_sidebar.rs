use super::Explorer;
use crate::{
    preferences::PreferenceError,
    ui::{self, ACCENT, BORDER, ERROR, HOVER, MUTED, PANEL, SELECTED, TEXT},
};
use gpui_kit::component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    tooltip::Tooltip,
};
use gpui_kit::{
    Context, Div, ElementId, Pixels, ScrollStrategy, Stateful, div, point, prelude::*, px, rgb,
    uniform_list,
};
use std::collections::BTreeSet;

pub(super) struct VisibleTable {
    name: String,
    favorite: bool,
}

fn table_element_id(role: &str, connection: crate::dynamodb::Connection, table: &str) -> ElementId {
    format!("{role}:{}:{table}", connection.label()).into()
}

struct ScrollAnchor {
    table: String,
    viewport_y: Pixels,
}

impl ScrollAnchor {
    fn capture(
        tables: &[VisibleTable],
        changed: &str,
        row_height: Pixels,
        offset_y: Pixels,
    ) -> Option<Self> {
        if tables.is_empty() || row_height <= px(0.) {
            return None;
        }
        let top = ((-offset_y / row_height).floor() as usize).min(tables.len() - 1);
        // The changed row may move or disappear. Keep a neighboring row in place.
        let index = (top..tables.len())
            .chain((0..top).rev())
            .find(|&index| tables[index].name != changed)?;
        Some(Self {
            table: tables[index].name.clone(),
            viewport_y: offset_y + row_height * index,
        })
    }

    fn offset_y(&self, tables: &[VisibleTable], row_height: Pixels) -> Option<Pixels> {
        let index = tables.iter().position(|table| table.name == self.table)?;
        Some(self.viewport_y - row_height * index)
    }
}

pub(super) fn visible_tables(
    tables: &[String],
    favorites: &BTreeSet<String>,
    query: &str,
    favorites_only: bool,
) -> Vec<VisibleTable> {
    let query = query.trim().to_lowercase();
    let mut visible: Vec<_> = tables
        .iter()
        .filter_map(|name| {
            let favorite = favorites.contains(name);
            if (favorites_only && !favorite) || !name.to_lowercase().contains(&query) {
                return None;
            }
            Some(VisibleTable {
                name: name.clone(),
                favorite,
            })
        })
        .collect();
    visible.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then_with(|| a.name.cmp(&b.name))
    });
    visible
}

impl Explorer {
    pub(super) fn rebuild_visible_tables(&mut self) {
        self.visible_tables = visible_tables(
            &self.tables,
            self.preferences.favorites(self.connection),
            &self.filter_text,
            self.favorites_only,
        );
    }

    pub(super) fn reset_table_list(&mut self) {
        self.rebuild_visible_tables();
        self.table_scroll
            .scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    fn toggle_favorite(&mut self, table: &str, cx: &mut Context<Self>) {
        if self.preference_store.is_none() {
            return;
        }
        let scroll = {
            let state = self.table_scroll.0.borrow();
            state.last_item_size.and_then(|size| {
                if self.visible_tables.is_empty() {
                    return None;
                }
                let row_height = size.contents.height / self.visible_tables.len() as f32;
                let anchor = ScrollAnchor::capture(
                    &self.visible_tables,
                    table,
                    row_height,
                    state.base_handle.offset().y,
                )?;
                Some((anchor, row_height, state.base_handle.clone()))
            })
        };
        self.preferences.toggle_favorite(self.connection, table);
        self.rebuild_visible_tables();
        if let Some((anchor, row_height, handle)) = scroll
            && let Some(offset_y) = anchor.offset_y(&self.visible_tables, row_height)
        {
            handle.set_offset(point(handle.offset().x, offset_y));
        }
        self.save_preferences(cx);
    }

    fn save_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.preference_store else {
            return;
        };
        self.save_id += 1;
        let save_id = self.save_id;
        let result = store.save(self.preferences.clone());
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let result = result.await.unwrap_or(Err(PreferenceError::Save));
            let _ = this.update(cx, |this, cx| {
                if this.save_id == save_id {
                    this.preference_error = result.err();
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn table_row(&self, index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let table = &self.visible_tables[index];
        let selected = self.selected_table.as_ref() == Some(&table.name);
        let name = table.name.clone();
        let favorite_name = name.clone();
        let tooltip_name = name.clone();
        div()
            .id(table_element_id("table", self.connection, &name))
            .h_10()
            .mb_1()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .bg(rgb(if selected { SELECTED } else { PANEL }))
            .text_color(rgb(if selected { ACCENT } else { TEXT }))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .child(
                div()
                    .id(table_element_id("table-name", self.connection, &name))
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(name.clone())
                    .tooltip(move |window, cx| {
                        Tooltip::new(tooltip_name.clone()).build(window, cx)
                    }),
            )
            .child(
                Button::new(table_element_id("favorite-table", self.connection, &name))
                    .ghost()
                    .small()
                    .icon(if table.favorite {
                        IconName::StarFill
                    } else {
                        IconName::Star
                    })
                    .text_color(rgb(if table.favorite { ACCENT } else { MUTED }))
                    .disabled(self.preference_store.is_none())
                    .tooltip(if table.favorite {
                        "Remove favorite"
                    } else {
                        "Add favorite"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favorite(&favorite_name, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.selected_table.as_ref() == Some(&name) {
                    return;
                }
                this.selected_table = Some(name.clone());
                this.browser.update(cx, |browser, cx| {
                    browser.open(this.connection, name.clone(), window, cx)
                });
                cx.notify();
            }))
    }

    pub(super) fn table_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut panel = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                        "TABLES ({} / {})",
                        self.visible_tables.len(),
                        self.tables.len()
                    )))
                    .child(ui::choice("refresh-tables", "Refresh", false).on_click(
                        cx.listener(|this, _, window, cx| this.refresh_tables(window, cx)),
                    )),
            )
            .child(
                Input::new(&self.table_filter)
                    .prefix(IconName::Search)
                    .cleanable(true),
            )
            .child(
                ui::choice("favorites-only", "Favorites only", self.favorites_only).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.favorites_only = !this.favorites_only;
                        this.reset_table_list();
                        cx.notify();
                    }),
                ),
            );
        if let Some(error) = self.preference_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(ERROR))
                    .child(error.message()),
            );
            if error == PreferenceError::Save {
                panel = panel.child(
                    ui::choice("retry-preferences", "Retry saving", false)
                        .on_click(cx.listener(|this, _, _, cx| this.save_preferences(cx))),
                );
            }
        }
        if !self.visible_tables.is_empty() {
            panel = panel.child(
                uniform_list(
                    "table-list",
                    self.visible_tables.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|index| this.table_row(index, cx)).collect()
                    }),
                )
                .flex_1()
                .min_h_0()
                .text_sm()
                .track_scroll(&self.table_scroll),
            );
        } else if !self.loading && self.error.is_none() {
            let message = if self.tables.is_empty() {
                "No tables in this connection"
            } else if self.favorites_only && self.filter_text.trim().is_empty() {
                "No favorite tables. Star a table to pin it here."
            } else {
                "No tables match your filter"
            };
            panel = panel.child(div().text_sm().text_color(rgb(MUTED)).child(message));
        }
        if self.loading {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Loading tables…"),
            );
        }
        if let Some(error) = self.error {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(rgb(ERROR)).child(error.title()))
                    .child(
                        div()
                            .text_color(rgb(MUTED))
                            .child(error.message(self.connection)),
                    ),
            );
        }
        panel
    }

    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> Div {
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
            .child(self.table_panel(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(visible: Vec<VisibleTable>) -> Vec<String> {
        visible.into_iter().map(|table| table.name).collect()
    }

    #[test]
    fn matches_case_insensitively_and_sorts_favorites_first() {
        let tables = ["OrdersProd", "OrdersDev", "Users", "AuditOrders"].map(String::from);
        let favorites = ["OrdersProd", "Users", "Missing"]
            .map(String::from)
            .into_iter()
            .collect();
        assert_eq!(
            names(visible_tables(&tables, &favorites, "  ORDERS ", false)),
            ["OrdersProd", "AuditOrders", "OrdersDev"]
        );
        assert_eq!(
            names(visible_tables(&tables, &favorites, "", true)),
            ["OrdersProd", "Users"]
        );
        assert_eq!(favorites.len(), 3, "Missing tables remain saved");
    }

    #[test]
    fn combines_name_and_favorites_filters_and_handles_no_matches() {
        let tables = ["Orders", "Users"].map(String::from);
        let favorites = ["Users".to_owned()].into_iter().collect();
        assert_eq!(
            names(visible_tables(&tables, &favorites, "us", true)),
            ["Users"]
        );
        assert!(visible_tables(&tables, &favorites, "Orders", true).is_empty());
        assert!(visible_tables(&[], &favorites, "", false).is_empty());
        assert_eq!(
            names(visible_tables(&tables, &favorites, "", false)),
            ["Users", "Orders"]
        );
    }

    #[test]
    fn favoriting_a_distant_row_keeps_the_scrolled_table_and_partial_offset() {
        let tables: Vec<_> = (0..100).map(|index| format!("Table{index:03}")).collect();
        let before = visible_tables(&tables, &BTreeSet::new(), "", false);
        let row_height = px(40.);
        let old_offset = px(-2013.);
        let anchor = ScrollAnchor::capture(&before, "Table055", row_height, old_offset).unwrap();
        let favorites = ["Table055".to_owned()].into_iter().collect();
        let after = visible_tables(&tables, &favorites, "", false);
        let offset = anchor.offset_y(&after, row_height).unwrap();
        assert_eq!(anchor.table, "Table050");
        assert_eq!(offset, px(-2053.));
        assert_eq!(offset + row_height * 51, px(-13.));

        // Removing a favorite above the viewport restores the original position.
        let anchor = ScrollAnchor::capture(&after, "Table055", row_height, offset).unwrap();
        assert_eq!(anchor.offset_y(&before, row_height), Some(old_offset));
    }

    #[test]
    fn removing_the_top_visible_favorite_keeps_its_neighbor_in_place() {
        let tables: Vec<_> = (0..100).map(|index| format!("Table{index:03}")).collect();
        let mut favorites: BTreeSet<_> = tables.iter().cloned().collect();
        let before = visible_tables(&tables, &favorites, "", true);
        let row_height = px(40.);
        let anchor = ScrollAnchor::capture(&before, "Table050", row_height, px(-2013.)).unwrap();
        favorites.remove("Table050");
        let after = visible_tables(&tables, &favorites, "", true);
        let offset = anchor.offset_y(&after, row_height).unwrap();
        assert_eq!(anchor.table, "Table051");
        assert_eq!(offset + row_height * 50, px(27.));
    }

    #[test]
    fn scroll_anchors_handle_unmeasured_and_empty_lists() {
        let tables = visible_tables(&["OnlyTable".into()], &BTreeSet::new(), "", false);
        assert!(ScrollAnchor::capture(&tables, "OnlyTable", px(40.), px(0.)).is_none());
        assert!(ScrollAnchor::capture(&tables, "OtherTable", px(0.), px(0.)).is_none());
        assert!(ScrollAnchor::capture(&[], "OtherTable", px(40.), px(0.)).is_none());
        let anchor = ScrollAnchor::capture(&tables, "OtherTable", px(40.), px(0.)).unwrap();
        assert!(anchor.offset_y(&[], px(40.)).is_none());
    }
}
