mod shell;

use gpui_kit::component::{ThemeRegistry, TitleBar};
use gpui_kit::*;

use crate::{features::settings::SettingsView, persistence::Preferences};

const BUNDLED_THEME_SETS: &[(&str, &str)] = &[
    (
        "adventure.json",
        include_str!("../../assets/themes/adventure.json"),
    ),
    (
        "alduin.json",
        include_str!("../../assets/themes/alduin.json"),
    ),
    (
        "asciinema.json",
        include_str!("../../assets/themes/asciinema.json"),
    ),
    (
        "aurora.json",
        include_str!("../../assets/themes/aurora.json"),
    ),
    ("ayu.json", include_str!("../../assets/themes/ayu.json")),
    (
        "catppuccin.json",
        include_str!("../../assets/themes/catppuccin.json"),
    ),
    (
        "everforest.json",
        include_str!("../../assets/themes/everforest.json"),
    ),
    (
        "fahrenheit.json",
        include_str!("../../assets/themes/fahrenheit.json"),
    ),
    (
        "flexoki.json",
        include_str!("../../assets/themes/flexoki.json"),
    ),
    (
        "gruvbox.json",
        include_str!("../../assets/themes/gruvbox.json"),
    ),
    (
        "harper.json",
        include_str!("../../assets/themes/harper.json"),
    ),
    (
        "hybrid.json",
        include_str!("../../assets/themes/hybrid.json"),
    ),
    (
        "jellybeans.json",
        include_str!("../../assets/themes/jellybeans.json"),
    ),
    (
        "kibble.json",
        include_str!("../../assets/themes/kibble.json"),
    ),
    (
        "macos-classic.json",
        include_str!("../../assets/themes/macos-classic.json"),
    ),
    (
        "mellifluous.json",
        include_str!("../../assets/themes/mellifluous.json"),
    ),
    (
        "molokai.json",
        include_str!("../../assets/themes/molokai.json"),
    ),
    (
        "solarized.json",
        include_str!("../../assets/themes/solarized.json"),
    ),
    (
        "spaceduck.json",
        include_str!("../../assets/themes/spaceduck.json"),
    ),
    (
        "tokyonight.json",
        include_str!("../../assets/themes/tokyonight.json"),
    ),
    (
        "twilight.json",
        include_str!("../../assets/themes/twilight.json"),
    ),
];

fn register_embedded_themes(cx: &mut App) {
    for &(filename, content) in BUNDLED_THEME_SETS {
        ThemeRegistry::global_mut(cx)
            .load_themes_from_str(content)
            .unwrap_or_else(|error| {
                panic!("Failed to register embedded theme '{filename}': {error}");
            });
    }
}

pub fn run() {
    application()
        .with_assets(crate::app_assets::AppAssets)
        .run(|cx| {
            init(cx);

            register_embedded_themes(cx);

            let preferences = Preferences::load();

            SettingsView::apply(preferences, None, cx);

            open_window(TitleBar::window_options(), cx, |window, cx| {
                window.set_window_title("Vermouth");

                cx.new(|cx| shell::AppShell::new(preferences, window, cx))
            })
            .expect("Could not open native Vermouth window");
        });
}
