use super::*;
use crate::dynamodb::{KeyDefinition, TableSchema};
use aws_sdk_dynamodb::types::ScalarAttributeType;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Bounds, Focusable, TestAppContext, WindowBounds, WindowOptions, px, size};

fn browser(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    Entity<ItemBrowser>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::actions::init(cx);
        let (window, browser) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(480.), px(470.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let history = cx.new(|cx| History::transient(window, cx));
                cx.new(|cx| {
                    let mut browser =
                        ItemBrowser::new(DynamoDbService::new().unwrap(), history, window, cx);
                    browser.state.schema = state::SchemaState::Ready {
                        table: "Events".into(),
                        schema: TableSchema {
                            targets: vec![KeyTarget {
                                index_name: Some("ByEventType".into()),
                                partition: KeyDefinition {
                                    name: "eventType".into(),
                                    kind: ScalarAttributeType::S,
                                },
                                sort: Some(KeyDefinition {
                                    name: "sk".into(),
                                    kind: ScalarAttributeType::N,
                                }),
                            }],
                        },
                    };
                    browser.inputs.sort_operator = crate::dynamodb::SortOperator::Between;
                    browser
                })
            },
        )
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), browser)
    })
}

#[gpui_kit::test]
fn enter_validates_each_query_input_and_shortcut_reveals_invalid_fields(cx: &mut TestAppContext) {
    let (window, browser) = browser(cx);
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.focus(window, cx));
        });
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "enter");
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            assert_eq!(
                browser.validation.as_deref(),
                Some("Enter a value for eventType.")
            );
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.set_value("UX_TEST", window, cx));
            browser.inputs.sort_input.update(cx, |input, cx| {
                input.set_value("1", window, cx);
                input.focus(window, cx);
            });
        });
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "ctrl-enter");
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            assert_eq!(browser.validation.as_deref(), Some("Enter a value for sk."));
            assert!(
                browser
                    .inputs
                    .end_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
    })
    .unwrap();
    browser.update(cx, |browser, _| browser.validation = None);
    cx.simulate_keystrokes(window.into(), "enter");
    browser.read_with(cx, |browser, _| {
        assert_eq!(browser.validation.as_deref(), Some("Enter a value for sk."))
    });
    cx.simulate_keystrokes(window.into(), "ctrl-shift-q");
    browser.read_with(cx, |browser, _| assert!(!browser.settings_expanded));
    cx.simulate_keystrokes(window.into(), "ctrl-enter");
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            assert!(browser.settings_expanded);
            assert!(
                browser
                    .inputs
                    .end_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert_eq!(browser.inputs.partition_input.read(cx).value(), "UX_TEST");
            assert!(browser.read_task.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn run_shortcut_does_not_confirm_scans_or_replace_inflight_queries(cx: &mut TestAppContext) {
    let (window, browser) = browser(cx);
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.mode = ReadMode::Scan;
            browser
                .state
                .reads
                .prepare_scan(browser.scan_request().unwrap(), None);
            browser.focus.focus(window, cx);
        });
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "ctrl-enter");
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            assert!(browser.state.reads.prepared_scan().is_some());
            assert!(!browser.state.reads.is_loading());
            browser.mode = ReadMode::Query;
            browser.inputs.sort_operator = crate::dynamodb::SortOperator::Any;
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.set_value("UX_TEST", window, cx));
            let request = browser.query_request(cx).unwrap();
            browser.state.reads.query(request).unwrap();
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        });
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "ctrl-enter");
    browser.read_with(cx, |browser, _| {
        assert!(browser.state.reads.is_loading());
        assert!(browser.validation.is_none());
        assert!(browser.read_task.is_none());
    });
}

#[gpui_kit::test]
fn constrained_layout_keeps_results_visible_and_long_keys_wrap(cx: &mut TestAppContext) {
    let (window, browser) = browser(cx);
    let long_key = "A".repeat(360);
    cx.update_window(window.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.inputs.sort_operator = crate::dynamodb::SortOperator::Any;
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.set_value("UX_TEST", window, cx));
            let request = browser.query_request(cx).unwrap();
            let pending = browser.state.reads.query(request).unwrap();
            browser.state.reads.finish(
                pending.id,
                Ok(crate::dynamodb::ItemPage {
                    items: vec![serde_json::json!({"eventType": long_key, "sk": "B".repeat(360)})],
                    next_key: None,
                }),
            );
            browser.inputs.sort_operator = crate::dynamodb::SortOperator::Between;
            browser.sync_results(window, cx);
        });
        window.draw(cx).clear(cx);
        let controls = browser.read(cx).settings_scroll.bounds();
        let results = window.find("results-pane").bounds();
        assert!(controls.size.height <= px(212.));
        assert!(results.size.height > px(140.), "results: {results:?}");
        let row = window.find("key-value:copy-partition-key:0").bounds();
        assert!(row.size.height > px(32.), "long key should wrap: {row:?}");
        window.click(("copy-partition-key", 0_usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), long_key);
        browser
            .read(cx)
            .result_panels
            .clone()
            .update(cx, |panels, cx| panels.resize(px(200.), cx));
        window.draw(cx).clear(cx);
        let width = browser.read(cx).result_panels.read(cx).width();
        assert!((width - px(200.)).abs() < px(1.));
        browser.update(cx, |browser, cx| browser.toggle_settings(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("results-pane").bounds().size.height > results.size.height);
        assert_eq!(browser.read(cx).result_panels.read(cx).width(), width);
        browser.update(cx, |browser, cx| browser.toggle_settings(window, cx));
        window.draw(cx).clear(cx);
        assert_eq!(browser.read(cx).result_panels.read(cx).width(), width);
        let divider = window.find("split-divider").bounds().center();
        window.drag(divider, gpui_kit::point(divider.x + px(30.), divider.y), cx);
        window.render_frame(cx);
        assert_eq!(
            browser.read(cx).result_panels.read(cx).width(),
            width + px(30.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn scan_page_has_real_overflow_and_scrollbar_drag_moves_content(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    let (handle, browser) = browser(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.mode = ReadMode::Scan;
            browser.settings_expanded = false;
            browser.state.reads.prepare_scan(browser.scan_request().unwrap(), None);
            let pending = browser.state.reads.confirm_scan().unwrap();
            browser.state.reads.finish(pending.id, Ok(crate::dynamodb::ItemPage {
                items: (0..50).map(|i| serde_json::json!({"eventType": format!("USER#{i}"), "sk": format!("EVENT#{i:04}")})).collect(),
                next_key: None,
            }));
            browser.sync_results(window, cx);
        });
        window.render_frame(cx);
        let viewport = browser.read(cx).results_list.viewport_bounds();
        let overlay = window.find("items-scrollbar-overlay").bounds();
        assert!(browser.read(cx).rendered_rows.get() < 50, "only visible rows should be built");
        assert!(window.try_find(("item", 49_usize)).is_none());
        assert!(browser.read(cx).results_list.max_offset_for_scrollbar().y > px(500.), "50 rows must overflow the viewport");
        // Use actual native wheel and pointer dispatch, rather than setting offsets directly.
        window.scroll("items-list", ScrollDelta::Pixels(point(px(0.), px(-150.))), cx);
        window.render_frame(cx);
        assert!(browser.read(cx).results_list.scroll_px_offset_for_scrollbar().y < px(0.));
        assert_eq!(window.find("items-scrollbar-overlay").bounds(), overlay, "the track must remain anchored to the viewport");
        let before = browser.read(cx).results_list.scroll_px_offset_for_scrollbar().y;
        window.drag(point(viewport.right() - px(8.), viewport.top() + px(60.)), point(viewport.right() - px(8.), viewport.bottom() - px(20.)), cx);
        window.render_frame(cx);
        assert!(browser.read(cx).results_list.scroll_px_offset_for_scrollbar().y < before, "dragging the scrollbar should advance the list");
        window.scroll("items-list", ScrollDelta::Pixels(point(px(0.), px(100_000.))), cx);
        window.render_frame(cx);
        window.drag(point(viewport.right() - px(8.), viewport.top() + px(8.)), point(viewport.right() - px(8.), viewport.bottom() + px(100.)), cx);
        window.render_frame(cx);
        let last_row = window.find(("item", 49_usize)).bounds();
        assert!(last_row.bottom() <= viewport.bottom() + px(1.), "dragging to the bottom must reveal the entire last row: {last_row:?}, viewport: {viewport:?}");
        let bottom = browser.read(cx).results_list.scroll_px_offset_for_scrollbar().y;
        window.scroll("items-list", ScrollDelta::Pixels(point(px(0.), px(-100_000.))), cx);
        window.render_frame(cx);
        assert_eq!(browser.read(cx).results_list.scroll_px_offset_for_scrollbar().y, bottom, "wheel scrolling must not move beyond the scrollbar's bottom");
        let list = &browser.read(cx).results_list;
        assert!((list.scroll_px_offset_for_scrollbar().y + list.max_offset_for_scrollbar().y).abs() <= px(1.), "the scrollbar range must include the full scrollable content");
    }).unwrap();
}

#[gpui_kit::test]
fn settled_result_split_does_not_emit_layout_notifications(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let (handle, browser) = browser(cx);
    let notifications = Rc::new(Cell::new(0));
    let count = notifications.clone();
    let subscription = browser.update(cx, |browser, cx| {
        cx.observe(&browser.result_panels, move |_, _, _| {
            count.set(count.get() + 1)
        })
    });
    cx.update_window(handle.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.inputs.sort_operator = crate::dynamodb::SortOperator::Any;
            browser
                .inputs
                .partition_input
                .update(cx, |input, cx| input.set_value("UX_TEST", window, cx));
            let pending = browser
                .state
                .reads
                .query(browser.query_request(cx).unwrap())
                .unwrap();
            browser.state.reads.finish(
                pending.id,
                Ok(crate::dynamodb::ItemPage {
                    items: vec![],
                    next_key: None,
                }),
            );
            browser.sync_results(window, cx);
        });
        for _ in 0..4 {
            window.render_frame(cx);
        }
    })
    .unwrap();
    let before = notifications.get();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        notifications.get(),
        before,
        "unchanged geometry must not notify on every paint"
    );
    drop(subscription);
}

#[gpui_kit::test]
fn scanned_long_pk_copies_and_pastes_exactly_across_input_focus_and_redraws(
    cx: &mut TestAppContext,
) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/dynamodb/items/ux-events-5.json"
    ))
    .unwrap();
    let item = fixture["Events"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|row| {
            let attributes = &row["PutRequest"]["Item"];
            let pk = attributes["pk"]["S"].as_str()?;
            pk.starts_with("UX#LONG#")
                .then(|| serde_json::json!({"pk": pk, "sk": attributes["sk"]["S"]}))
        })
        .unwrap();
    let pk = item["pk"].as_str().unwrap().to_owned();
    assert_eq!(pk.len(), 388);
    let (handle, browser) = browser(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.state.schema = state::SchemaState::Ready {
                table: "Events".into(),
                schema: TableSchema {
                    targets: vec![KeyTarget {
                        index_name: None,
                        partition: KeyDefinition {
                            name: "pk".into(),
                            kind: ScalarAttributeType::S,
                        },
                        sort: Some(KeyDefinition {
                            name: "sk".into(),
                            kind: ScalarAttributeType::S,
                        }),
                    }],
                },
            };
            browser.inputs.sort_operator = crate::dynamodb::SortOperator::Any;
            browser.mode = ReadMode::Scan;
            browser
                .state
                .reads
                .prepare_scan(browser.scan_request().unwrap(), None);
            let pending = browser.state.reads.confirm_scan().unwrap();
            browser.state.reads.finish(
                pending.id,
                Ok(crate::dynamodb::ItemPage {
                    items: vec![item],
                    next_key: None,
                }),
            );
            browser.sync_results(window, cx);
        });
        window.render_frame(cx);
        window.click(("copy-partition-key", 0_usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), pk);
        window.click("Query", cx);
        window.render_frame(cx);
        window.click("query-partition-input", cx);
        window.set_modifiers(gpui_kit::Keystroke::parse("ctrl-v").unwrap().modifiers);
        window.press("ctrl-v", cx);
        window.set_modifiers(Default::default());
        assert_eq!(
            browser
                .read(cx)
                .inputs
                .partition_input
                .read(cx)
                .value()
                .as_ref(),
            pk
        );
        for _ in 0..4 {
            window.press("ctrl-a", cx);
            window.set_modifiers(gpui_kit::Keystroke::parse("ctrl-v").unwrap().modifiers);
            window.press("ctrl-v", cx);
            window.set_modifiers(Default::default());
            window.render_frame(cx);
            assert_eq!(
                browser
                    .read(cx)
                    .inputs
                    .partition_input
                    .read(cx)
                    .value()
                    .as_ref(),
                pk
            );
        }
        // Repeated pointer hover and focus changes must not insert or rotate text.
        for _ in 0..4 {
            window.click("query-partition-input", cx);
            window.hover("query-partition-input", cx);
            window.render_frame(cx);
        }
    })
    .unwrap();
    for _ in 0..4 {
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let request = browser.update(cx, |view, cx| {
                assert_eq!(view.inputs.partition_input.read(cx).value().as_ref(), pk);
                view.query_request(cx).unwrap()
            });
            assert!(request.target.index_name.is_none());
            let ReadOperation::Query { partition, sort } = request.operation else {
                panic!("expected query");
            };
            assert_eq!(
                partition,
                aws_sdk_dynamodb::types::AttributeValue::S(pk.clone())
            );
            assert!(sort.is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn paste_repeats_stop_when_the_shortcut_modifier_is_released(cx: &mut TestAppContext) {
    use gpui_kit::{
        ClipboardItem, KeyDownEvent, Keystroke, Modifiers, ModifiersChangedEvent, PlatformInput,
    };
    let (handle, browser) = browser(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        let input = browser.read(cx).inputs.partition_input.clone();
        input.update(cx, |input, cx| input.focus(window, cx));
        window.render_frame(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("UX#PAGINATION".into()));
        let key = Keystroke::parse("ctrl-v").unwrap();
        window.dispatch_event(
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers: key.modifiers,
                capslock: Default::default(),
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: key.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        assert_eq!(input.read(cx).value(), "UX#PAGINATION");
        window.dispatch_event(
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers: Modifiers::default(),
                capslock: Default::default(),
            }),
            cx,
        );
        // Wayland's repeat timer retains the original shortcut modifiers.
        // Simulate a missed key release followed by modifier release.
        for _ in 0..3 {
            window.dispatch_event(
                PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: key.clone(),
                    is_held: true,
                    prefer_character_input: false,
                }),
                cx,
            );
        }
        assert_eq!(input.read(cx).value(), "UX#PAGINATION");
        window.dispatch_event(
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers: key.modifiers,
                capslock: Default::default(),
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: key.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        assert_eq!(input.read(cx).value(), "UX#PAGINATIONUX#PAGINATION");
    })
    .unwrap();
}
