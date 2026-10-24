mod shell;

use std::path::PathBuf;

use gpui_kit::component::{ThemeRegistry, TitleBar};
use gpui_kit::*;

use crate::{features::settings::SettingsView, persistence::Preferences};

pub fn run() {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);

        let preferences = Preferences::load();
        SettingsView::apply(preferences, None, cx);

        if let Err(error) = ThemeRegistry::watch_dir(PathBuf::from("assets/themes"), cx, |cx| {
            SettingsView::apply(Preferences::load(), None, cx);
        }) {
            eprintln!("Theme directory watcher unavailable: {error}");
        }

        open_window(TitleBar::window_options(), cx, |window, cx| {
            cx.new(|cx| shell::AppShell::new(preferences, window, cx))
        })
        .expect("Could not open native Vermouth window");
    });
}
