mod models;
mod store;

use crate::{
    browser::item_detail::ItemDetail,
    dynamodb::Connection,
    ui::{self, BORDER, ERROR, MUTED, PANEL, SELECTED},
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AppContext, Context, Entity, EventEmitter, IntoElement, Render, Subscription, Task, Window,
    div, prelude::*, px, rgb, uniform_list,
};
pub use models::{Entry, SavedQuery};
use serde_json::Value;
use store::{Command, HistoryStore, Reply};

pub struct RestoreQuery {
    pub table: String,
    pub query: SavedQuery,
}

pub struct History {
    store: Option<HistoryStore>,
    entries: Vec<Entry>,
    selected: Option<String>,
    detail: Entity<ItemDetail>,
    filter: Entity<InputState>,
    filter_text: String,
    error: Option<String>,
    mutation_error: Option<String>,
    skipped_files: usize,
    loading: bool,
    revision: u64,
    selection_id: u64,
    payload_task: Option<Task<()>>,
    pending_delete: Option<Vec<String>>,
    _filter_subscription: Subscription,
}

impl EventEmitter<RestoreQuery> for History {}

impl History {
    pub fn has_error(&self) -> bool {
        self.error.is_some() || self.mutation_error.is_some() || self.skipped_files > 0
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_store(HistoryStore::open(), window, cx)
    }

    fn with_store(
        store: std::io::Result<HistoryStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search history by table, query or connection…")
        });
        let subscription = cx.subscribe(&filter, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter_text = input.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        let (store, error) = match store {
            Ok(store) => (Some(store), None),
            Err(error) => {
                log::warn!("Couldn't open history: {error}");
                (None, Some("History storage is unavailable. Restart after checking the app's data folder.".into()))
            }
        };
        let mut this = Self {
            store,
            entries: Vec::new(),
            selected: None,
            detail: cx.new(|cx| ItemDetail::new(window, cx)),
            filter,
            filter_text: String::new(),
            error,
            mutation_error: None,
            skipped_files: 0,
            loading: false,
            revision: 0,
            selection_id: 0,
            payload_task: None,
            pending_delete: None,
            _filter_subscription: subscription,
        };
        this.command(Command::List, cx);
        this
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        let Some(store) = &self.store else { return };
        let mutation = matches!(command, Command::Save { .. } | Command::Delete { .. });
        let reply = store.send(command);
        self.revision += 1;
        let revision = self.revision;
        self.loading = true;
        // File operations live in the worker, independent of the UI task lifetime.
        cx.spawn(async move |this, cx| {
            let result = reply.await.unwrap_or_else(|_| {
                Err("History worker stopped. Restart the app to retry.".into())
            });
            let _ = this.update(cx, |this, cx| {
                // Stale listings may be ignored, but every failed write must be
                // reported. Refresh and unrelated saves cannot retry a lost change.
                if mutation && let Err(error) = &result {
                    this.mutation_error = Some(error.clone());
                    cx.notify();
                }
                if revision != this.revision {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(Reply::Entries(listing)) => {
                        this.entries = listing.entries;
                        this.skipped_files = listing.skipped_files;
                        this.error = None;
                        if this
                            .selected
                            .as_ref()
                            .is_some_and(|id| !this.entries.iter().any(|entry| &entry.id == id))
                        {
                            this.selected = None;
                            this.payload_task = None;
                            this.detail.update(cx, |detail, cx| detail.clear(cx));
                        }
                    }
                    Err(error) if !mutation => this.error = Some(error),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn record(&mut self, entry: Entry, create: bool, cx: &mut Context<Self>) {
        self.command(
            Command::Save {
                entry: Box::new(entry),
                payload: None,
                create,
            },
            cx,
        );
    }

    pub fn save_snapshot(
        &mut self,
        connection: Connection,
        table: String,
        item: Value,
        cx: &mut Context<Self>,
    ) {
        let mut entry = Entry::new(connection, table, None);
        entry.snapshot = true;
        self.command(
            Command::Save {
                entry: Box::new(entry),
                payload: Some(item),
                create: true,
            },
            cx,
        );
    }

    fn select(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.payload_task = None;
        self.selection_id += 1;
        let selection = self.selection_id;
        self.selected = Some(entry.id.clone());
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        if entry.snapshot
            && let Some(store) = &self.store
        {
            let reply = store.send(Command::Payload(entry.id));
            self.payload_task = Some(cx.spawn_in(window, async move |this, cx| {
                let result = reply.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.selection_id != selection || this.selected.is_none() {
                        return;
                    }
                    match result {
                        Ok(Ok(Reply::Payload(value))) => this.detail.update(cx, |detail, cx| {
                            detail.set_item(Some(((selection, 0), &value)), window, cx)
                        }),
                        _ => {
                            this.error = Some(
                                "Saved item couldn't be loaded. Check its history file and retry."
                                    .into(),
                            )
                        }
                    }
                    this.payload_task = None;
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }
}

impl Render for History {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut toolbar =
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(Input::new(&self.filter))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(ui::choice("history-refresh", "Refresh", false).on_click(
                            cx.listener(|this, _, _, cx| this.command(Command::List, cx)),
                        ))
                        .child(
                            ui::choice("history-delete-old", "Delete older than 30 days", false)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let cutoff =
                                        models::now_ms().saturating_sub(30 * 24 * 60 * 60 * 1000);
                                    this.pending_delete = Some(
                                        this.entries
                                            .iter()
                                            .filter(|entry| entry.created_ms < cutoff)
                                            .map(|entry| entry.id.clone())
                                            .collect(),
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(
                            ui::choice("history-delete-all", "Delete all", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.pending_delete = Some(
                                        this.entries.iter().map(|entry| entry.id.clone()).collect(),
                                    );
                                    cx.notify();
                                }),
                            ),
                        ),
                );
        if let Some(ids) = &self.pending_delete {
            toolbar = toolbar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(format!("Permanently delete {} history entries?", ids.len()))
                    .child(
                        ui::choice("history-confirm-delete", "Delete", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(ids) = this.pending_delete.take() {
                                    this.command(Command::Delete { ids }, cx);
                                }
                            }),
                        ),
                    )
                    .child(
                        ui::choice("history-cancel-delete", "Cancel", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.pending_delete = None;
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        if let Some(error) = &self.error {
            toolbar = toolbar.child(div().text_color(rgb(ERROR)).child(error.clone()));
        }
        if let Some(error) = &self.mutation_error {
            toolbar = toolbar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(ERROR))
                            .child(format!("A history change failed. {error}")),
                    )
                    .child(
                        ui::choice("history-dismiss-error", "Dismiss", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.mutation_error = None;
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        if self.skipped_files > 0 {
            toolbar = toolbar.child(div().text_sm().text_color(rgb(ERROR)).child(format!(
                "{} history files couldn't be loaded and have been kept. Other entries are available. Check the terminal for file details.", self.skipped_files
            )));
        }
        let visible: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let text = format!(
                    "{} {} {}",
                    entry.table,
                    entry.connection,
                    entry
                        .query
                        .as_ref()
                        .map_or("Saved item", |query| query.summary.as_str())
                );
                text.to_lowercase()
                    .contains(&self.filter_text)
                    .then_some(index)
            })
            .collect();
        let list = uniform_list(
            "history-list",
            visible.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let entry = &this.entries[visible[index]];
                        let selected = this.selected.as_ref() == Some(&entry.id);
                        let entry = entry.clone();
                        let date = chrono::DateTime::from_timestamp_millis(entry.created_ms as i64)
                            .map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_default();
                        div()
                            .id(format!("history-{}", entry.id))
                            .h(px(112.0))
                            .p_3()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(if selected { SELECTED } else { PANEL }))
                            .cursor_pointer()
                            .child(div().text_ellipsis().child(format!(
                                "{} · {}",
                                entry.table,
                                if entry.snapshot {
                                    "Saved item"
                                } else {
                                    "Query"
                                }
                            )))
                            .child(div().text_sm().text_color(rgb(MUTED)).child(date))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .text_ellipsis()
                                    .child(entry.connection.clone()),
                            )
                            .when_some(entry.query.as_ref(), |row, query| {
                                row.child(
                                    div().text_sm().text_ellipsis().child(query.summary.clone()),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(entry.clone(), window, cx)
                            }))
                    })
                    .collect()
            }),
        )
        .w_80()
        .flex_shrink_0()
        .min_h_0()
        .border_r_1()
        .border_color(rgb(BORDER));
        let mut detail = div().flex().flex_col().flex_1().min_w_0().min_h_0();
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| Some(&entry.id) == self.selected.as_ref())
            .cloned()
        {
            if !entry.snapshot {
                let value = serde_json::json!({"query": entry.query, "result_count": entry.result_count, "pages_fetched": entry.pages, "primary_keys": entry.keys});
                self.detail.update(cx, |detail, cx| {
                    detail.set_item(Some(((self.selection_id, entry.pages), &value)), window, cx)
                });
            }
            let id = entry.id.clone();
            let mut actions = div().flex().gap_2().p_3();
            if let Some(query) = entry.query {
                actions = actions.child(
                    ui::choice("restore-query", "Restore query", false).on_click(cx.listener(
                        move |_, _, _, cx| {
                            cx.emit(RestoreQuery {
                                table: entry.table.clone(),
                                query: query.clone(),
                            });
                        },
                    )),
                );
            }
            actions = actions.child(
                ui::choice("delete-history-entry", "Delete entry", false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.command(
                            Command::Delete {
                                ids: vec![id.clone()],
                            },
                            cx,
                        );
                    },
                )),
            );
            detail = detail.child(actions);
            if self.payload_task.is_some() {
                detail = detail.child(
                    div()
                        .p_3()
                        .text_color(rgb(MUTED))
                        .child("Loading saved item…"),
                );
            } else {
                detail = detail.child(div().flex_1().min_h_0().child(self.detail.clone()));
            }
        } else {
            detail = detail.child(ui::empty_state("Query history and saved items", "Select an entry to inspect or restore it. Restore uses your current connection and does not run the query."));
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(toolbar)
            .when(self.loading && self.entries.is_empty(), |view| {
                view.child(div().p_3().text_color(rgb(MUTED)).child("Loading history…"))
            })
            .child(div().flex().flex_1().min_h_0().child(list).child(detail))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    #[gpui_kit::test]
    fn failed_save_stays_visible_after_newer_refresh_and_successful_save(cx: &mut TestAppContext) {
        // A real filesystem worker may wake tasks from another thread.
        cx.background_executor.allow_parking();
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let store = HistoryStore::open_at(directory.path().to_owned()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (history, visual) =
            cx.add_window_view(move |window, cx| History::with_store(Ok(store), window, cx));
        let mut failed = Entry::new(Connection::Local, "Books".into(), None);
        failed.snapshot = true;
        // A directory at the payload path forces a real atomic-write failure,
        // while metadata listing remains readable (also works when tests run as root).
        std::fs::create_dir(directory.path().join(format!("{}.item.json", failed.id))).unwrap();
        VisualTestContext::update(visual, |_, cx| {
            history.update(cx, |history, cx| {
                history.command(
                    Command::Save {
                        entry: Box::new(failed),
                        payload: Some(serde_json::json!({"id": "lost"})),
                        create: true,
                    },
                    cx,
                );
                history.command(Command::List, cx);
            });
        });
        let barrier =
            visual.read(|cx| history.read(cx).store.as_ref().unwrap().send(Command::List));
        runtime.block_on(barrier).unwrap().unwrap();
        visual.run_until_parked();
        visual.read(|cx| {
            let history = history.read(cx);
            assert!(history.mutation_error.is_some());
            assert!(history.error.is_none());
            assert!(history.entries.is_empty());
            assert!(history.has_error());
            assert!(!history.loading);
        });
        VisualTestContext::update(visual, |_, cx| {
            history.update(cx, |history, cx| {
                history.record(
                    Entry::new(Connection::Local, "Books".into(), None),
                    true,
                    cx,
                );
                history.command(Command::List, cx);
            });
        });
        let barrier =
            visual.read(|cx| history.read(cx).store.as_ref().unwrap().send(Command::List));
        runtime.block_on(barrier).unwrap().unwrap();
        visual.run_until_parked();
        VisualTestContext::update(visual, |window, cx| {
            history.update(cx, |history, cx| {
                assert_eq!(history.entries.len(), 1);
                assert!(history.mutation_error.is_some());
                history.mutation_error = None;
                assert!(!history.has_error());
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
    }

    #[gpui_kit::test]
    fn saved_items_load_offline_and_query_details_follow_pagination(cx: &mut TestAppContext) {
        cx.background_executor.allow_parking();
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let store = HistoryStore::open_at(directory.path().to_owned()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let payload = serde_json::json!({"id": "saved", "historical_value": "original"});
        let mut snapshot = Entry::new(Connection::Local, "Books".into(), None);
        snapshot.snapshot = true;
        runtime
            .block_on(store.send(Command::Save {
                entry: Box::new(snapshot.clone()),
                payload: Some(payload.clone()),
                create: true,
            }))
            .unwrap()
            .unwrap();
        let (history, visual) =
            cx.add_window_view(move |window, cx| History::with_store(Ok(store), window, cx));
        // A worker barrier makes the test independent of filesystem thread scheduling.
        let barrier =
            visual.read(|cx| history.read(cx).store.as_ref().unwrap().send(Command::List));
        runtime.block_on(barrier).unwrap().unwrap();
        visual.run_until_parked();
        visual.read(|cx| assert_eq!(history.read(cx).entries.len(), 1));
        VisualTestContext::update(visual, |window, cx| {
            history.update(cx, |history, cx| history.select(snapshot, window, cx))
        });
        let barrier =
            visual.read(|cx| history.read(cx).store.as_ref().unwrap().send(Command::List));
        runtime.block_on(barrier).unwrap().unwrap();
        visual.run_until_parked();
        VisualTestContext::update(visual, |window, cx| {
            let this = history.read(cx);
            assert!(this.error.is_none());
            let json = this.detail.read(cx).displayed_json().unwrap();
            assert_eq!(serde_json::from_str::<Value>(json).unwrap(), payload);
            window.draw(cx).clear(cx);
        });

        let mut query = Entry::new(Connection::Local, "Books".into(), None);
        query.pages = 1;
        VisualTestContext::update(visual, |window, cx| {
            history.update(cx, |history, cx| {
                history.entries.push(query.clone());
                history.select(query.clone(), window, cx);
            });
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        VisualTestContext::update(visual, |window, cx| {
            history.update(cx, |history, cx| {
                let entry = history
                    .entries
                    .iter_mut()
                    .find(|entry| entry.id == query.id)
                    .unwrap();
                entry.pages = 2;
                entry.result_count = 70;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        visual.read(|cx| {
            let json = history.read(cx).detail.read(cx).displayed_json().unwrap();
            let value: Value = serde_json::from_str(json).unwrap();
            assert_eq!(value["pages_fetched"], 2);
            assert_eq!(value["result_count"], 70);
        });
    }
}
