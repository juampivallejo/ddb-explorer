//! Application shortcuts. Views own their handlers and execution guards.
use gpui_kit::{App, KeyBinding, actions};

actions!(
    explorer,
    [
        RunQuery,
        ToggleSidebar,
        ToggleQuerySettings,
        FocusTableFilter
    ]
);

pub fn init(cx: &mut App) {
    // Wayland repeats retain the original modifiers even after their release.
    // Reject stale paste shortcuts before they dispatch actions and hide the pointer.
    cx.intercept_keystrokes(|event, window, cx| {
        let key = &event.keystroke;
        let modifiers = window.modifiers();
        if key.key == "v" && key.modifiers.secondary() && !modifiers.secondary() {
            cx.stop_propagation();
        }
    })
    .detach();
    // Match input contexts at their own depth so editor bindings cannot consume
    // workspace shortcuts (especially Ctrl+Enter in the JSON viewer).
    for context in ["DdbExplorer", "DdbExplorer > Input"] {
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", RunQuery, Some(context)),
            KeyBinding::new("ctrl-b", ToggleSidebar, Some(context)),
            KeyBinding::new("ctrl-shift-q", ToggleQuerySettings, Some(context)),
            KeyBinding::new("ctrl-l", FocusTableFilter, Some(context)),
        ]);
    }
}
