mod app;
mod dynamodb;
mod ui;

use app::Explorer;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(Bounds::maximized(None, cx))),
                window_min_size: Some(size(px(800.0), px(600.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("DynamoDB Explorer".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Explorer::default()),
        )
        .expect("failed to open the gpui-demo window");

        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });
}
