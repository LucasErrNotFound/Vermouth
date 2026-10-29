use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Theme, ThemeRegistry, WindowExt as _,
    button::Button, switch::Switch, theme::ThemeMode,
};
use gpui_kit::*;

use crate::persistence::{Appearance, Preferences, ThemeFamily, set_run_at_startup};

pub struct SettingsView {
    prefs: Preferences,
    save_error: Option<String>,
}

impl SettingsView {
    pub fn new(prefs: Preferences) -> Self {
        Self {
            prefs,
            save_error: None,
        }
    }

    pub fn preferences(&self) -> Preferences {
        self.prefs
    }

    pub fn has_variant(family: ThemeFamily, dark: bool, cx: &App) -> bool {
        family.has_local_palette()
            || ThemeRegistry::global(cx)
                .themes()
                .keys()
                .any(|name| family.matches_registry_name(name.as_ref(), dark))
    }

    pub fn apply(prefs: Preferences, window: Option<&mut Window>, cx: &mut App) {
        match prefs.appearance {
            Appearance::System => {
                Theme::sync_system_appearance(window, cx);
            }
            Appearance::Light => {
                Theme::change(ThemeMode::Light, window, cx);
            }
            Appearance::Dark => {
                Theme::change(ThemeMode::Dark, window, cx);
            }
        }

        let dark = Theme::global(cx).mode.is_dark();

        let family = if Self::has_variant(prefs.family, dark, cx) {
            prefs.family
        } else {
            ThemeFamily::Default
        };

        let config = ThemeRegistry::global(cx)
            .themes()
            .iter()
            .find(|(name, _)| family.matches_registry_name(name.as_ref(), dark))
            .map(|(_, config)| config.clone());

        let palette = Palette::for_family(family, dark);

        Theme::update(cx, |theme| {
            if let Some(config) = config {
                theme.apply_config(&config);
            } else {
                theme.background = palette.background.into();
                theme.foreground = palette.foreground.into();
                theme.sidebar = palette.sidebar.into();
                theme.sidebar_foreground = palette.foreground.into();
                theme.sidebar_border = palette.border.into();
                theme.border = palette.border.into();
                theme.group_box = palette.surface.into();
                theme.group_box_foreground = palette.foreground.into();
                theme.popover = palette.background.into();
                theme.popover_foreground = palette.foreground.into();
                theme.secondary = palette.surface.into();
                theme.secondary_foreground = palette.foreground.into();
                theme.muted = palette.surface.into();
                theme.muted_foreground = palette.muted.into();
                theme.input = palette.border.into();
                theme.ring = palette.accent.into();
                theme.accent = palette.surface.into();
                theme.accent_foreground = palette.foreground.into();
                theme.primary = palette.accent.into();

                theme.primary_foreground = if dark {
                    rgb(0x15191f).into()
                } else {
                    rgb(0xffffff).into()
                };

                theme.sidebar_accent = palette.surface.into();
                theme.sidebar_accent_foreground = palette.foreground.into();
            }

            theme.font_family = if cfg!(target_os = "windows") {
                "Segoe UI".into()
            } else if cfg!(target_os = "macos") {
                ".SystemUIFont".into()
            } else {
                "Noto Sans".into()
            };

            theme.font_size = px(if prefs.compact { 15. } else { 16. });

            theme.notification.placement = Anchor::BottomRight;
        });
    }

    pub fn is_dark(&self, cx: &App) -> bool {
        Theme::global(cx).mode.is_dark()
    }

    fn persist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_error = self
            .prefs
            .save()
            .err()
            .map(|_| "Could not save settings to disk.".to_owned());

        if let Some(message) = self.save_error.clone() {
            window.push_notification(
                gpui_kit::component::notification::Notification::error(message)
                    .autohide(false)
                    .placement(Anchor::BottomRight),
                cx,
            );
        }

        cx.notify();
    }

    pub fn set_appearance(
        &mut self,
        selected: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prefs.appearance = selected;
        Self::apply(self.prefs, Some(window), cx);

        if !Self::has_variant(self.prefs.family, self.is_dark(cx), cx) {
            self.prefs.family = ThemeFamily::Default;
            Self::apply(self.prefs, Some(window), cx);
        }

        self.persist(window, cx);
    }

    pub fn set_family(
        &mut self,
        selected: ThemeFamily,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_variant(selected, self.is_dark(cx), cx) {
            window.push_notification(
                gpui_kit::component::notification::Notification::info(format!(
                    "{} is unavailable. Add its GPUI-Kit JSON to themes/.",
                    selected.variant_name(self.is_dark(cx))
                ))
                .placement(Anchor::BottomRight),
                cx,
            );
            return;
        }

        self.prefs.family = selected;
        Self::apply(self.prefs, Some(window), cx);
        self.persist(window, cx);
    }

    pub fn set_density(&mut self, compact: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.compact = compact;
        Self::apply(self.prefs, Some(window), cx);
        self.persist(window, cx);
    }

    pub fn set_startup(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        match set_run_at_startup(enabled) {
            Ok(()) => {
                self.prefs.run_at_startup = enabled;
                self.persist(window, cx);

                window.push_notification(
                    gpui_kit::component::notification::Notification::success(if enabled {
                        "Vermouth will launch at Windows sign-in."
                    } else {
                        "Launch at sign-in disabled."
                    })
                    .placement(Anchor::BottomRight),
                    cx,
                );
            }
            Err(_) => {
                window.push_notification(
                    gpui_kit::component::notification::Notification::error(
                        "Could not change startup registration. Your setting was not changed.",
                    )
                    .autohide(false)
                    .placement(Anchor::BottomRight),
                    cx,
                );
            }
        }

        cx.notify();
    }
}

struct Palette {
    background: Rgba,
    foreground: Rgba,
    sidebar: Rgba,
    surface: Rgba,
    border: Rgba,
    muted: Rgba,
    accent: Rgba,
}

impl Palette {
    fn for_family(family: ThemeFamily, dark: bool) -> Self {
        let (background, foreground, sidebar, surface, border, muted, accent) = match (family, dark)
        {
            (ThemeFamily::Default, false) => (
                0xffffff, 0x17202d, 0xf7f8fa, 0xf2f4f7, 0xe1e5eb, 0x697586, 0x3668c9,
            ),
            (ThemeFamily::Default, true) => (
                0x15191f, 0xf4f6fa, 0x1b2028, 0x242b35, 0x343d4b, 0xabb5c4, 0x91b5ff,
            ),
            (ThemeFamily::Gruvbox, false) => (
                0xfbf1c7, 0x3c3836, 0xf2e5bc, 0xebdbb2, 0xd5c4a1, 0x665c54, 0x9d0006,
            ),
            (ThemeFamily::Gruvbox, true) => (
                0x282828, 0xebdbb2, 0x32302f, 0x3c3836, 0x504945, 0xa89984, 0xfabd2f,
            ),
            (ThemeFamily::Catppuccin, false) => (
                0xeff1f5, 0x4c4f69, 0xe6e9ef, 0xdce0e8, 0xccd0da, 0x6c6f85, 0x8839ef,
            ),
            (ThemeFamily::Catppuccin, true) => (
                0x1e1e2e, 0xcdd6f4, 0x181825, 0x313244, 0x45475a, 0xa6adc8, 0xcba6f7,
            ),
            (ThemeFamily::Ayu, false) => (
                0xfcfcfc, 0x5c6166, 0xf3f4f5, 0xecedf0, 0xd8dce0, 0x7c858d, 0xf29718,
            ),
            (ThemeFamily::Ayu, true) => (
                0x0b0e14, 0xbfbdb6, 0x11151c, 0x1b2028, 0x2d3540, 0x9aa2ac, 0xe6b450,
            ),
            (ThemeFamily::Graphite, false) => (
                0xf8f9fa, 0x202226, 0xedefef, 0xe3e5e8, 0xd2d5d9, 0x6d737c, 0x596b80,
            ),
            (ThemeFamily::Graphite, true) => (
                0x161718, 0xe5e7e9, 0x202225, 0x292c30, 0x383d42, 0xa6adb4, 0x9baabd,
            ),
            (_, false) => (
                0xffffff, 0x17202d, 0xf7f8fa, 0xf2f4f7, 0xe1e5eb, 0x697586, 0x3668c9,
            ),
            (_, true) => (
                0x15191f, 0xf4f6fa, 0x1b2028, 0x242b35, 0x343d4b, 0xa6adb4, 0x91b5ff,
            ),
        };

        Self {
            background: rgb(background),
            foreground: rgb(foreground),
            sidebar: rgb(sidebar),
            surface: rgb(surface),
            border: rgb(border),
            muted: rgb(muted),
            accent: rgb(accent),
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.is_dark(cx);

        let available_themes = ThemeFamily::ALL
            .into_iter()
            .filter(|family| Self::has_variant(*family, dark, cx))
            .collect::<Vec<_>>();

        let appearance_buttons = [Appearance::System, Appearance::Light, Appearance::Dark]
            .into_iter()
            .map(|choice| {
                Button::new(format!("appearance-{}", choice.label()))
                    .label(if self.prefs.appearance == choice {
                        format!("✓ {}", choice.label())
                    } else {
                        choice.label().to_owned()
                    })
                    .cursor_pointer()
                    .tooltip(format!("Use {} appearance", choice.label()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(choice, window, cx);
                    }))
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .max_w(px(760.))
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(section_title(
                                "Appearance", cx,
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(
                                        "Use the system appearance or select a specific mode.",
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(appearance_buttons),
                            )
                            .child(section_title(
                                "Themes", cx,
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(format!(
                                        "Current: {}",
                                        self.prefs.family.variant_name(dark)
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(
                                        available_themes
                                            .into_iter()
                                            .map(|family| {
                                                Button::new(format!(
                                                    "theme-page-{}",
                                                    family.name()
                                                ))
                                                .icon(IconName::Palette)
                                                .label(
                                                    family.variant_name(dark),
                                                )
                                                .selected(
                                                    self.prefs.family == family,
                                                )
                                                .cursor_pointer()
                                                .tooltip(format!(
                                                    "Apply the {} theme",
                                                    family.variant_name(dark)
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.set_family(
                                                            family, window, cx,
                                                        );
                                                    },
                                                ))
                                            }),
                                    ),
                            )
                            .child(section_title(
                                "Interface density", cx,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(
                                            "density-comfortable",
                                        )
                                        .label(if self.prefs.compact {
                                            "Comfortable"
                                        } else {
                                            "✓ Comfortable"
                                        })
                                        .cursor_pointer()
                                        .tooltip(
                                            "Use more spacing between interface controls",
                                        )
                                        .on_click(cx.listener(
                                            |this, _, window, cx| {
                                                this.set_density(
                                                    false, window, cx,
                                                );
                                            },
                                        )),
                                    )
                                    .child(
                                        Button::new(
                                            "density-compact",
                                        )
                                        .label(if self.prefs.compact {
                                            "✓ Compact"
                                        } else {
                                            "Compact"
                                        })
                                        .cursor_pointer()
                                        .tooltip(
                                            "Fit more controls into smaller windows",
                                        )
                                        .on_click(cx.listener(
                                            |this, _, window, cx| {
                                                this.set_density(
                                                    true, window, cx,
                                                );
                                            },
                                        )),
                                    ),
                            )
                            .child(section_title(
                                "Startup", cx,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                "Run at startup",
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(
                                                        cx.theme().muted_foreground,
                                                    )
                                                    .child(
                                                        "Windows: launches the current executable at sign-in.",
                                                    ),
                                            ),
                                    )
                                    .child(
                                        Switch::new(
                                            "startup-settings",
                                        )
                                        .accessibility_label(
                                            "Run at startup",
                                        )
                                        .when(
                                            cfg!(target_os = "windows"),
                                            |control| {
                                                control.cursor_pointer()
                                            },
                                        )
                                        .tooltip(
                                            "Launch Vermouth automatically when you sign in to Windows",
                                        )
                                        .checked(
                                            self.prefs.run_at_startup,
                                        )
                                        .disabled(
                                            !cfg!(target_os = "windows"),
                                        )
                                        .on_change(cx.listener(
                                            |this, checked, window, cx| {
                                                this.set_startup(
                                                    *checked, window, cx,
                                                );
                                            },
                                        )),
                                    ),
                            )
                            .when_some(
                                self.save_error.clone(),
                                |view, error| {
                                    view.child(
                                        div()
                                            .text_sm()
                                            .text_color(
                                                cx.theme().danger,
                                            )
                                            .child(error),
                                    )
                                },
                            )
                            .child(
                                div()
                                    .pt_3()
                                    .text_sm()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(
                                        "Only installed themes compatible with the current appearance are shown. Saved cameras start disconnected when Vermouth launches.",
                                    ),
                            ),
                    ),
            )
    }
}

fn section_title(label: &'static str, cx: &Context<SettingsView>) -> AnyElement {
    div()
        .pt_4()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().foreground)
        .child(label)
        .into_any_element()
}
