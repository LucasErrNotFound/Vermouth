use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, Collapsible as _, Disableable as _, Icon, Selectable as _, Sizable as _,
    TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
    popover::Popover,
    scroll::ScrollableElement as _,
    sidebar::{
        Sidebar, SidebarGroup, SidebarItem, SidebarMenu, SidebarMenuItem, SidebarToggleButton,
    },
    switch::Switch,
    tooltip::Tooltip,
};
use gpui_kit::*;

use crate::app_assets::APP_ICON_PATH;

use crate::{
    camera::CameraCatalog,
    features::{
        cameras::CamerasView, diagnostics::DiagnosticsView, live_view::LiveView,
        recordings::RecordingsView, settings::SettingsView,
    },
    persistence::{Appearance, Preferences, ThemeFamily},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Live,
    Cameras,
    Recordings,
    Diagnostics,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Live => "Live View",
            Self::Cameras => "Cameras",
            Self::Recordings => "Recordings",
            Self::Diagnostics => "Diagnostics",
            Self::Settings => "Settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Live => IconName::Video,
            Self::Cameras => IconName::Camera,
            Self::Recordings => IconName::HardDrive,
            Self::Diagnostics => IconName::Activity,
            Self::Settings => IconName::Settings,
        }
    }
}

pub struct AppShell {
    page: Page,
    live: Entity<LiveView>,
    cameras: Entity<CamerasView>,
    recordings: Entity<RecordingsView>,
    diagnostics: Entity<DiagnosticsView>,
    settings: Entity<SettingsView>,
    sidebar_preference: Option<bool>,
    _settings_observer: Subscription,
}

impl AppShell {
    pub fn new(prefs: Preferences, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalog = cx.new(|_| CameraCatalog::new());
        let settings = cx.new(|_| SettingsView::new(prefs));

        let settings_observer = cx.observe(&settings, |_, _, cx| cx.notify());

        let cameras = cx.new(|cx| CamerasView::new(catalog.clone(), window, cx));

        let live = cx.new(|cx| LiveView::new(catalog.clone(), cameras.clone(), window, cx));

        Self {
            page: Page::Live,
            live,
            cameras,
            recordings: cx.new(|_| RecordingsView),
            diagnostics: cx.new(|_| DiagnosticsView),
            settings,
            sidebar_preference: None,
            _settings_observer: settings_observer,
        }
    }

    fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            if page != Page::Live {
                self.live.update(cx, |live, cx| live.stop_ptz_control(cx));
            }
            self.page = page;
            cx.notify();
        }
    }

    fn toggle_sidebar(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        let next = !collapsed;

        self.sidebar_preference = Some(next);

        self.live.update(cx, |live, cx| {
            live.set_navigation_collapsed(next, cx);
        });

        cx.notify();
    }

    fn sidebar_item(&self, page: Page, cx: &mut Context<Self>) -> SidebarMenuItem {
        SidebarMenuItem::new(page.label())
            .icon(page.icon())
            .active(self.page == page)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate(page, cx);
            }))
    }

    fn sidebar(
        &self,
        collapsed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Sidebar<SidebarGroup<SidebarMenu>> {
        let sidebar_foreground = cx.theme().sidebar_foreground;

        let main_menu = SidebarMenu::new()
            .child(self.sidebar_item(Page::Live, cx))
            .child(self.sidebar_item(Page::Cameras, cx))
            .child(self.sidebar_item(Page::Recordings, cx));

        let secondary_menu = SidebarMenu::new()
            .child(self.sidebar_item(Page::Diagnostics, cx))
            .child(self.sidebar_item(Page::Settings, cx));

        let toggle_label = if collapsed {
            "Expand sidebar"
        } else {
            "Collapse sidebar"
        };

        let toggle = SidebarToggleButton::new()
            .collapsed(collapsed)
            .accessibility_label(toggle_label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_sidebar(collapsed, cx);
            }));

        let header = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .when(!collapsed, |header| {
                header.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(sidebar_foreground)
                        .child("Navigation"),
                )
            })
            .child(
                div()
                    .id("sidebar-collapse-control")
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(toggle_label).build(window, cx))
                    .child(toggle),
            );

        let footer_menu =
            secondary_menu
                .collapsed(collapsed)
                .render("vermouth-secondary-navigation", window, cx);

        Sidebar::new("vermouth-sidebar")
            .collapsible(true)
            .collapsed(collapsed)
            .w(px(218.))
            .header(header)
            .child(SidebarGroup::new("Monitoring").child(main_menu))
            .footer(
                div()
                    .id("vermouth-secondary-navigation-container")
                    .w_full()
                    .px_2()
                    .pb_3()
                    .child(footer_menu),
            )
    }

    fn set_dark(&mut self, checked: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.update(cx, |view, cx| {
            view.set_appearance(
                if checked {
                    Appearance::Dark
                } else {
                    Appearance::Light
                },
                window,
                cx,
            );
        });
    }

    fn set_startup(&mut self, checked: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.update(cx, |view, cx| {
            view.set_startup(checked, window, cx);
        });
    }

    fn set_family(&mut self, family: ThemeFamily, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.update(cx, |view, cx| {
            view.set_family(family, window, cx);
        });
    }

    fn settings_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let prefs = settings.preferences();
        let dark = settings.is_dark(cx);

        let mode_tooltip = if dark {
            "Switch to light appearance"
        } else {
            "Switch to dark appearance"
        };

        let startup_supported = cfg!(target_os = "windows");

        let startup_tooltip = if !startup_supported {
            "Run at startup is currently supported on Windows only"
        } else if prefs.run_at_startup {
            "Stop launching Vermouth automatically when you sign in"
        } else {
            "Launch Vermouth when you sign in to Windows"
        };

        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

        let available_themes = ThemeFamily::ALL
            .into_iter()
            .filter(|family| SettingsView::has_variant(*family, dark, cx))
            .collect::<Vec<_>>();

        let theme_rows = available_themes
            .into_iter()
            .map(|family| {
                let selected = prefs.family == family;
                let name = family.variant_name(dark);

                Button::new(format!("popover-theme-{}", family.name(),))
                    .ghost()
                    .w_full()
                    .flex_shrink_0()
                    .justify_start()
                    .cursor_pointer()
                    .selected(selected)
                    .accessibility_label(name.clone())
                    .tooltip(format!("Apply the {name} theme"))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Palette).small())
                            .child(div().flex_1().min_w_0().truncate().child(name))
                            .child(
                                div()
                                    .w(px(18.))
                                    .h(px(18.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |slot| {
                                        slot.child(Icon::new(IconName::Check).small())
                                    }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_family(family, window, cx);
                    }))
            })
            .collect::<Vec<_>>();

        Popover::new("vermouth-appearance-popover")
            .anchor(Anchor::TopRight)
            .offset(px(6.))
            .trigger(
                Button::new("titlebar-settings")
                    .ghost()
                    .cursor_pointer()
                    .icon(IconName::Settings)
                    .tooltip("Appearance, themes and startup"),
            )
            .child(
                div()
                    .w(rems(17.5))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child("Dark mode")
                            .child(div().flex_1())
                            .child(
                                Switch::new("titlebar-dark-mode")
                                    .cursor_pointer()
                                    .tooltip(mode_tooltip)
                                    .accessibility_label("Dark mode")
                                    .checked(dark)
                                    .on_change(cx.listener(|this, checked, window, cx| {
                                        this.set_dark(*checked, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child("Run at startup")
                                    .child(div().flex_1())
                                    .child(
                                        Switch::new("titlebar-run-at-startup")
                                            .when(startup_supported, |control| {
                                                control.cursor_pointer()
                                            })
                                            .tooltip(startup_tooltip)
                                            .accessibility_label("Run at startup")
                                            .checked(prefs.run_at_startup)
                                            .disabled(!startup_supported)
                                            .on_change(cx.listener(|this, checked, window, cx| {
                                                this.set_startup(*checked, window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(if startup_supported {
                                        "Launch when you sign in to Windows."
                                    } else {
                                        "Available on Windows."
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(border)
                            .pt_3()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Themes"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(format!("Current: {}", prefs.family.variant_name(dark))),
                            ),
                    )
                    .child(
                        div().w_full().h(rems(20.)).flex_shrink_0().child(
                            div()
                                .id("titlebar-theme-list")
                                .size_full()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .pr_2()
                                .children(theme_rows)
                                .overflow_y_scrollbar(),
                        ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self
            .sidebar_preference
            .unwrap_or(window.viewport_size().width < px(960.));

        let content: AnyElement = match self.page {
            Page::Live => self.live.clone().into_any_element(),
            Page::Cameras => self.cameras.clone().into_any_element(),
            Page::Recordings => self.recordings.clone().into_any_element(),
            Page::Diagnostics => self.diagnostics.clone().into_any_element(),
            Page::Settings => self.settings.clone().into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new()
                    .bg(cx.theme().sidebar)
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                img(APP_ICON_PATH)
                                    .w(px(26.))
                                    .h(px(26.))
                                    .flex_shrink_0()
                                    .object_fit(ObjectFit::Contain),
                            )
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Vermouth"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.settings_menu(cx))
                            .child(
                                Button::new("titlebar-github")
                                    .ghost()
                                    .icon(IconName::Github)
                                    .cursor_pointer()
                                    .tooltip("GitHub repository URL has not been configured")
                                    .on_click(|_, window, cx| {
                                        window.push_notification(
                                            Notification::info(
                                                "GitHub repository URL is not set yet.",
                                            )
                                            .placement(Anchor::BottomRight),
                                            cx,
                                        );
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(self.sidebar(collapsed, window, cx))
                    .child(div().flex_1().min_w_0().min_h_0().child(content)),
            )
    }
}
