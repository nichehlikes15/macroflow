// On Windows, MacroFlow is a windowed app: no console window opens behind it.
// (`macroflow run …` attaches to the terminal it was started from instead.)
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod assets;
mod cli;
mod engine;
mod keys;
mod marker;
mod model;
mod platform;
mod remote;
mod ui;
mod update;
mod vision;

use gpui::{
    actions, point, prelude::*, px, size, App, Application, Bounds, KeyBinding, Menu, MenuItem,
    TitlebarOptions, WindowBounds, WindowOptions,
};

actions!(macroflow, [Quit, CheckForUpdates]);

/// If MacroFlow itself crashes, say so with a red pop-up. The pop-up is a separate
/// little program, so it still appears even though MacroFlow is closing.
fn install_crash_alert() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SHOWN: AtomicBool = AtomicBool::new(false);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Crashes while a macro plays are caught and reported by the macro runner.
        let on_run_thread = std::thread::current().name() == Some(engine::RUN_THREAD);
        if !on_run_thread && !SHOWN.swap(true, Ordering::SeqCst) {
            let why = info
                .payload()
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".into());
            let place = info
                .location()
                .map(|l| format!(" ({}:{})", l.file(), l.line()))
                .unwrap_or_default();
            let _ = vision::show_notification(&vision::Toast {
                title: "MacroFlow crashed".into(),
                message: format!("{why}{place}. Any running macro has stopped."),
                style: "error",
                position: "top-center",
                duration_ms: 15000,
                wait: false,
                sound: true,
            });
        }
        default_hook(info);
    }));
}

fn main() {
    // `macroflow run my-macro.json` plays a macro without the editor.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        platform::attach_console();
    }
    if let Some(code) = cli::main(&args) {
        std::process::exit(code);
    }
    install_crash_alert();
    // The old version an in-app update moved aside isn't needed any more.
    update::clean_up_after_update();
    Application::new().with_assets(assets::Assets).run(|cx: &mut App| {
        #[cfg(target_os = "macos")]
        assets::set_dock_icon();
        // The editor handles Quit itself so it can ask to save first; this is the fallback.
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "MacroFlow".into(),
            items: vec![
                MenuItem::action("Check for Updates…", CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Quit MacroFlow", Quit),
            ],
        }]);
        // Quit when the last window closes (the area-selection window closing doesn't count).
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1360.), px(840.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // The system title bar is transparent so MacroFlow's own dark title bar
                // shows through. On macOS the traffic lights are centred in it; on Windows
                // MacroFlow draws its own minimise / maximise / close buttons.
                titlebar: Some(TitlebarOptions {
                    title: Some("MacroFlow".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(ui::TITLEBAR_H / 2. - 6.))),
                }),
                window_min_size: Some(size(px(960.), px(600.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| ui::MacroApp::new(window, cx)),
        )
        .expect("failed to open the MacroFlow window");
        cx.activate(true);
    });
}
