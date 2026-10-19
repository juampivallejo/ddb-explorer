use super::*;
use crate::dynamodb::{KeyDefinition, TableSchema};
use aws_sdk_dynamodb::types::ScalarAttributeType;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, px, size};

fn browser(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    Entity<ItemBrowser>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
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
fn scan_page_has_real_overflow_and_scrollbar_drag_moves_content(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    let (handle, browser) = browser(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        browser.update(cx, |browser, cx| {
            browser.mode = ReadMode::Scan;
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
