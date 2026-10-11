mod app;
mod dynamodb;
mod ui;

use app::Explorer;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

fn main() {
    // GPUI reports platform/event-loop failures through the log facade.
    log::set_logger(&ErrorLogger).expect("failed to initialize logging");
    log::set_max_level(log::LevelFilter::Warn);
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
            |_, cx| cx.new(Explorer::new),
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

struct ErrorLogger;

impl log::Log for ErrorLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}
