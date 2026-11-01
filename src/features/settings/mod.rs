use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Theme, ThemeRegistry, WindowExt as _,
    button::Button, notification::Notification, switch::Switch, theme::ThemeMode,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::persistence::{
    Appearance, Preferences, set_run_at_startup,
    theme_catalog::{self, ThemeDescriptor},
};

pub struct SettingsView {
    prefs: Preferences,
    save_error: Option<String>,
}

impl SettingsView {
    pub fn new(prefs: Preferences, cx: &mut App) -> Self {
        theme_catalog::ensure_registered(cx);

        Self {
            prefs,
            save_error: None,
        }
    }

    pub fn preferences(&self) -> Preferences {
        self.prefs
    }

    pub fn available_themes(cx: &App, dark: bool) -> Vec<(usize, &'static ThemeDescriptor)> {
        theme_catalog::available(cx, dark)
    }

    pub fn current_theme_index(&self, cx: &App) -> usize {
        let dark = self.is_dark(cx);

        theme_catalog::resolve(cx, self.prefs.theme_index(dark), dark)
    }

    pub fn current_theme_name(&self, cx: &App) -> String {
        let index = self.current_theme_index(cx);

        theme_catalog::get(index)
            .map(|theme| theme.display_name.clone())
            .unwrap_or_else(|| "Default".to_owned())
    }

    pub fn apply(prefs: Preferences, window: Option<&mut Window>, cx: &mut App) {
        theme_catalog::ensure_registered(cx);

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

        let index = theme_catalog::resolve(cx, prefs.theme_index(dark), dark);

        let config = theme_catalog::get(index).and_then(|theme| {
            ThemeRegistry::global(cx)
                .themes()
                .get(theme.id.as_str())
                .cloned()
        });

        Theme::update(cx, |theme| {
            if let Some(config) = config {
                theme.apply_config(&config);
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
                Notification::error(message)
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
        self.persist(window, cx);
    }

    pub fn set_theme(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let dark = self.is_dark(cx);

        let available = Self::available_themes(cx, dark);

        let valid = available
            .iter()
            .any(|(available_index, _)| *available_index == index);

        if !valid {
            window.push_notification(
                Notification::info("This theme is unavailable for the current appearance.")
                    .placement(Anchor::BottomRight),
                cx,
            );
            return;
        }

        self.prefs.set_theme_index(dark, index);

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
                    Notification::success(if enabled {
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
                    Notification::error(
                        "Could not change startup registration. \
                         Your setting was not changed.",
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

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.is_dark(cx);

        let themes = Self::available_themes(cx, dark);
        let selected_index = self.current_theme_index(cx);
        let current_name = self.current_theme_name(cx);

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

        let theme_buttons = themes
            .into_iter()
            .map(|(index, theme)| {
                let name = theme.display_name.clone();

                Button::new(format!("theme-page-{index}"))
                    .icon(IconName::Palette)
                    .label(name.clone())
                    .selected(selected_index == index)
                    .cursor_pointer()
                    .tooltip(format!("Apply {name}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_theme(index, window, cx);
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
                            .child(
                                section_title("Appearance", cx),
                            )
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
                            .child(
                                section_title("Themes", cx),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(format!(
                                        "Current: {current_name}"
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(theme_buttons),
                            )
                            .child(
                                section_title(
                                    "Interface density",
                                    cx,
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(
                                            "density-comfortable",
                                        )
                                        .label(
                                            if self.prefs.compact {
                                                "Comfortable"
                                            } else {
                                                "✓ Comfortable"
                                            },
                                        )
                                        .cursor_pointer()
                                        .tooltip(
                                            "Use more spacing between interface controls",
                                        )
                                        .on_click(
                                            cx.listener(
                                                |this, _, window, cx| {
                                                    this.set_density(
                                                        false,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ),
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            "density-compact",
                                        )
                                        .label(
                                            if self.prefs.compact {
                                                "✓ Compact"
                                            } else {
                                                "Compact"
                                            },
                                        )
                                        .cursor_pointer()
                                        .tooltip(
                                            "Fit more controls into smaller windows",
                                        )
                                        .on_click(
                                            cx.listener(
                                                |this, _, window, cx| {
                                                    this.set_density(
                                                        true,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ),
                                        ),
                                    ),
                            )
                            .child(
                                section_title("Startup", cx),
                            )
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
                                            .child("Run at startup")
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(
                                                        cx.theme()
                                                            .muted_foreground,
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
                                            cfg!(
                                                target_os = "windows"
                                            ),
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
                                            !cfg!(
                                                target_os = "windows"
                                            ),
                                        )
                                        .on_change(
                                            cx.listener(
                                                |this, checked, window, cx| {
                                                    this.set_startup(
                                                        *checked,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ),
                                        ),
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
                                        "Themes are loaded from the bundled JSON catalog. Only variants compatible with the current appearance are displayed.",
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
