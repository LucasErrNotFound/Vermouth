use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

pub mod layout;

use gpui_kit::component::{ActiveTheme as _, Sizable as _, button::Button, tooltip::Tooltip};
use gpui_kit::*;

use crate::camera::{Camera, CameraCatalog, CameraId, ConnectionState};
use layout::{WindowClass, grid_columns, window_class};

pub struct LiveView {
    catalog: Entity<CameraCatalog>,
    selected: Option<CameraId>,
    focused: bool,
    inspector: bool,
    navigation_collapsed: Option<bool>,
    _observer: Subscription,
}

impl LiveView {
    pub fn new(catalog: Entity<CameraCatalog>, cx: &mut Context<Self>) -> Self {
        let selected = catalog.read(cx).all().first().map(|camera| camera.id);

        let observer = cx.observe(&catalog, |this, catalog, cx| {
            if this
                .selected
                .is_some_and(|id| catalog.read(cx).get(id).is_none())
            {
                this.selected = catalog.read(cx).all().first().map(|camera| camera.id);
            }

            cx.notify();
        });

        Self {
            catalog,
            selected,
            focused: false,
            inspector: true,
            navigation_collapsed: None,
            _observer: observer,
        }
    }

    pub(crate) fn set_navigation_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.navigation_collapsed = Some(collapsed);
        cx.notify();
    }

    fn select(&mut self, id: CameraId, cx: &mut Context<Self>) {
        self.selected = Some(id);
        cx.notify();
    }

    fn tile(camera: Camera, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = camera.id;
        let connection_tip = camera.connection.description();
        let card_tip = format!("Select {}", camera.name);

        let accent_border = if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        };

        let state_color = match camera.connection {
            ConnectionState::PreviewUnavailable => cx.theme().info,
            ConnectionState::Connecting => cx.theme().warning,
            ConnectionState::Offline => cx.theme().muted_foreground,
            ConnectionState::Unauthorized | ConnectionState::Unsupported => cx.theme().danger,
        };

        div()
            .id(format!("camera-tile-{}", id.0))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(card_tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id, cx);
            }))
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .rounded_lg()
            .bg(cx.theme().group_box)
            .border_1()
            .border_color(accent_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(camera.name),
                    )
                    .child(
                        div()
                            .id(format!("camera-status-{}", id.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .tooltip(move |window, cx| {
                                Tooltip::new(connection_tip).build(window, cx)
                            })
                            .child(
                                div()
                                    .w(px(7.))
                                    .h(px(7.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(state_color),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if selected { "SELECTED" } else { "DEMO" }),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(16.0 / 9.0)
                    .bg(rgb(0x11171e))
                    .text_color(rgb(0xc1ccd7))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child("NO VIDEO")
                    .child(div().text_xs().child("Preview intentionally unavailable")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(state_color)
                            .truncate()
                            .child(camera.connection.label()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if selected {
                                "Selected"
                            } else {
                                "Click to select"
                            }),
                    ),
            )
            .into_any_element()
    }

    fn inspector(camera: &Camera, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_lg()
            .bg(cx.theme().group_box)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Camera inspector"),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(camera.name.clone()),
            )
            .child(info_row(
                "Connection",
                camera.connection.label(),
                cx,
            ))
            .child(info_row(
                "Transport",
                camera.transport.label(),
                cx,
            ))
            .child(info_row(
                "PTZ",
                camera.ptz.label(),
                cx,
            ))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(camera.connection.description()),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Controls and snapshot are unavailable until a real authorized camera backend is connected.",
                    ),
            )
            .into_any_element()
    }
}

fn info_row(label: &'static str, value: &'static str, cx: &Context<LiveView>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_sm().child(value))
        .into_any_element()
}

impl Render for LiveView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logical_width = window.viewport_size().width.as_f32();
        let window_class = window_class(logical_width);
        let narrow = window_class == WindowClass::Narrow;

        let docked_inspector = logical_width >= 1500.0 && self.inspector;

        let cameras = self.catalog.read(cx).all().to_vec();

        let selected = self
            .selected
            .filter(|id| cameras.iter().any(|camera| camera.id == *id))
            .or_else(|| cameras.first().map(|camera| camera.id));

        let chosen = selected
            .and_then(|id| cameras.iter().find(|camera| camera.id == id))
            .cloned();

        let navigation_collapsed = self.navigation_collapsed.unwrap_or(logical_width < 960.);

        let content_width = logical_width
            - if navigation_collapsed { 62. } else { 218. }
            - if docked_inspector { 340. } else { 0. };

        let columns = grid_columns(
            content_width.max(260.),
            cameras.len(),
            self.focused || narrow,
        );

        let show_chooser = (self.focused || narrow) && cameras.len() > 1;

        let chooser_buttons = if show_chooser {
            cameras
                .iter()
                .map(|camera| {
                    let id = camera.id;

                    Button::new(format!("choose-{}", id.0))
                        .label(if Some(id) == selected {
                            format!("✓ {}", camera.name)
                        } else {
                            camera.name.clone()
                        })
                        .small()
                        .cursor_pointer()
                        .tooltip(format!("Select {} in the demo workspace", camera.name))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select(id, cx);
                        }))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let visible: Vec<_> = if self.focused || narrow {
            cameras
                .iter()
                .filter(|camera| Some(camera.id) == selected)
                .cloned()
                .collect()
        } else {
            cameras.clone()
        };

        let rows: Vec<_> = visible
            .chunks(columns)
            .map(|group| {
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .children(group.iter().map(|camera| {
                        div().flex_1().min_w_0().child(Self::tile(
                            camera.clone(),
                            Some(camera.id) == selected,
                            cx,
                        ))
                    }))
            })
            .collect();

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
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Live View"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Demo workspace · no cameras are connected"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("focus-view")
                                    .small()
                                    .icon(IconName::Video)
                                    .label(if self.focused { "Grid" } else { "Focus" })
                                    .cursor_pointer()
                                    .tooltip(if self.focused {
                                        "Show the camera grid"
                                    } else {
                                        "Focus on the selected camera"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.focused = !this.focused;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("inspector-toggle")
                                    .small()
                                    .icon(IconName::Eye)
                                    .label(if self.inspector {
                                        "Hide details"
                                    } else {
                                        "Details"
                                    })
                                    .cursor_pointer()
                                    .tooltip(if self.inspector {
                                        "Close the camera inspector"
                                    } else {
                                        "Show connection and capability details"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.inspector = !this.inspector;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(show_chooser, |root| {
                root.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .px_3()
                        .pt_3()
                        .children(chooser_buttons),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .gap_3()
                    .p_3()
                    .child(
                        div()
                            .id("live-camera-scroll")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .when(visible.is_empty(), |view| {
                                        view.child(empty_workspace(cx))
                                    })
                                    .children(rows),
                            ),
                    )
                    .when(docked_inspector, |body| {
                        body.child(
                            div()
                                .w(px(300.))
                                .flex_shrink_0()
                                .id("live-inspector-scroll")
                                .overflow_y_scroll()
                                .children(
                                    chosen.as_ref().map(|camera| Self::inspector(camera, cx)),
                                ),
                        )
                    }),
            )
            .when(
                self.inspector && !docked_inspector && chosen.is_some(),
                |root| {
                    root.child(
                        div()
                            .px_3()
                            .pb_3()
                            .max_h(px(220.))
                            .id("compact-inspector-scroll")
                            .overflow_y_scroll()
                            .children(chosen.as_ref().map(|camera| Self::inspector(camera, cx))),
                    )
                },
            )
    }
}

fn empty_workspace(cx: &Context<LiveView>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .p_8()
        .text_color(cx.theme().muted_foreground)
        .child("No cameras configured")
        .child("Open Cameras to add a demo camera configuration.")
        .into_any_element()
}
