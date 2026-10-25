use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, button::Button, spinner::Spinner, tooltip::Tooltip,
};
use gpui_kit::*;

pub mod layout;

use crate::camera::{Camera, CameraCatalog, CameraId, ConnectionState};

use layout::{WindowClass, grid_columns, window_class};

pub struct LiveView {
    catalog: Entity<CameraCatalog>,
    selected: Option<CameraId>,
    focused: bool,
    inspector: bool,
    navigation_collapsed: Option<bool>,
    frames: HashMap<CameraId, (u64, Arc<RenderImage>)>,
    retired: VecDeque<(Instant, Arc<RenderImage>)>,
    _observer: Subscription,
    _poll_task: Task<()>,
}

impl LiveView {
    pub fn new(catalog: Entity<CameraCatalog>, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&catalog, |view, catalog, cx| {
            let all = catalog.read(cx).all();

            if view
                .selected
                .is_none_or(|id| !all.iter().any(|camera| camera.id == id))
            {
                view.selected = all.first().map(|camera| camera.id);
            }

            cx.notify();
        });

        let task = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(67))
                    .await;

                if weak.update(cx, |view, cx| view.tick(cx)).is_err() {
                    break;
                }
            }
        });

        Self {
            catalog,
            selected: None,
            focused: false,
            inspector: true,
            navigation_collapsed: None,
            frames: HashMap::new(),
            retired: VecDeque::new(),
            _observer: observer,
            _poll_task: task,
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.catalog.update(cx, |catalog, cx| {
            if catalog.poll() {
                cx.notify();
            }
        });

        let cameras = self.catalog.read(cx).all().to_vec();
        let now = Instant::now();
        let mut changed = false;

        let removed = self
            .frames
            .keys()
            .copied()
            .filter(|id| !cameras.iter().any(|camera| camera.id == *id))
            .collect::<Vec<_>>();

        for id in removed {
            if let Some((_, old)) = self.frames.remove(&id) {
                self.retired.push_back((now, old));
                changed = true;
            }
        }

        for camera in &cameras {
            let Some(frame) = camera.latest_frame.as_ref() else {
                if let Some((_, old)) = self.frames.remove(&camera.id) {
                    self.retired.push_back((now, old));
                    changed = true;
                }

                continue;
            };

            let current = self
                .frames
                .get(&camera.id)
                .is_some_and(|(revision, _)| *revision == camera.frame_revision);

            if current {
                continue;
            }

            if let Some((_, old)) = self
                .frames
                .insert(camera.id, (camera.frame_revision, frame.clone()))
            {
                self.retired.push_back((now, old));
            }

            changed = true;
        }

        while self
            .retired
            .front()
            .is_some_and(|(time, _)| now.duration_since(*time) >= Duration::from_secs(1))
        {
            if let Some((_, image)) = self.retired.pop_front() {
                cx.drop_image(image, None);
            }
        }

        if changed {
            cx.notify();
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

    fn status(camera: &Camera, cx: &Context<Self>) -> AnyElement {
        let color = match camera.connection {
            ConnectionState::Offline => cx.theme().muted_foreground,
            ConnectionState::Connecting => cx.theme().warning,
            ConnectionState::Live => cx.theme().success,
            ConnectionState::Stale => cx.theme().warning,
            ConnectionState::Error => cx.theme().danger,
        };

        let icon: AnyElement = match camera.connection {
            ConnectionState::Offline => Icon::new(IconName::WifiOff)
                .small()
                .text_color(color)
                .into_any_element(),
            ConnectionState::Connecting => Spinner::new().small().color(color).into_any_element(),
            ConnectionState::Live => Icon::new(IconName::CircleCheck)
                .small()
                .text_color(color)
                .into_any_element(),
            ConnectionState::Stale => Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(color)
                .into_any_element(),
            ConnectionState::Error => Icon::new(IconName::CircleX)
                .small()
                .text_color(color)
                .into_any_element(),
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(color)
            .child(icon)
            .child(camera.connection.label())
            .into_any_element()
    }

    fn tile(&mut self, camera: Camera, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = camera.id;

        let frame = self.frames.get(&id).map(|(_, frame)| frame.clone());

        let border = if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        };

        let status = Self::status(&camera, cx);

        let middle: AnyElement = if let Some(frame) = frame {
            img(frame)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(match camera.connection {
                    ConnectionState::Offline => "Disconnected",
                    ConnectionState::Connecting => "Connecting to camera...",
                    ConnectionState::Live => "Preparing video...",
                    ConnectionState::Stale => "Waiting for video...",
                    ConnectionState::Error => "Video unavailable",
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x94a3b8))
                        .child(camera.detail.unwrap_or(camera.connection.description())),
                )
                .into_any_element()
        };

        let name = camera.name;
        let tip = format!("Select {name}");

        div()
            .id(format!("camera-tile-{}", id.0))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.select(id, cx);
            }))
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .rounded_lg()
            .bg(cx.theme().group_box)
            .border_1()
            .border_color(border)
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
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if selected { "SELECTED" } else { "RTSP" }),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .aspect_ratio(16.0 / 9.0)
                    .overflow_hidden()
                    .bg(rgb(0x11171e))
                    .text_color(rgb(0xc1ccd7))
                    .child(middle),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(status)
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

    fn inspector(&self, camera: &Camera, cx: &Context<Self>) -> AnyElement {
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
            .child(Self::status(camera, cx))
            .child(info_row("Transport", "RTSP / TCP", cx))
            .child(info_row("PTZ", "Not available", cx))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(camera.detail.unwrap_or(camera.connection.description())),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No recording, audio, ONVIF or PTZ in this release."),
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
        let width = window.viewport_size().width.as_f32();
        let narrow = window_class(width) == WindowClass::Narrow;

        let docked = width >= 1500.0 && self.inspector;
        let cameras = self.catalog.read(cx).all().to_vec();

        let selected = self
            .selected
            .filter(|id| cameras.iter().any(|camera| camera.id == *id))
            .or_else(|| cameras.first().map(|camera| camera.id));

        let chosen = selected
            .and_then(|id| cameras.iter().find(|camera| camera.id == id))
            .cloned();

        let nav_collapsed = self.navigation_collapsed.unwrap_or(width < 960.0);

        let content_width =
            width - if nav_collapsed { 62.0 } else { 218.0 } - if docked { 340.0 } else { 0.0 };

        let columns = grid_columns(
            content_width.max(260.0),
            cameras.len(),
            self.focused || narrow,
        );

        let show_chooser = (self.focused || narrow) && cameras.len() > 1;

        let chooser = if show_chooser {
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
                        .tooltip(format!("Select {}", camera.name))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select(id, cx);
                        }))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let visible = if self.focused || narrow {
            cameras
                .iter()
                .filter(|camera| Some(camera.id) == selected)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            cameras.clone()
        };

        let mut rows = Vec::new();

        for group in visible.chunks(columns) {
            let tiles = group
                .iter()
                .map(|camera| {
                    let tile = self.tile(camera.clone(), Some(camera.id) == selected, cx);

                    div().flex_1().min_w_0().child(tile)
                })
                .collect::<Vec<_>>();

            rows.push(div().flex().items_start().gap_3().children(tiles));
        }

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
                                    .font_weight(
                                        FontWeight::SEMIBOLD,
                                    )
                                    .child("Live View"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(
                                        "Authorized RTSP camera streams",
                                    ),
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
                                    .label(if self.focused {
                                        "Grid"
                                    } else {
                                        "Focus"
                                    })
                                    .cursor_pointer()
                                    .tooltip(
                                        "Toggle focused camera layout",
                                    )
                                    .on_click(cx.listener(
                                        |view, _, _, cx| {
                                            view.focused =
                                                !view.focused;
                                            cx.notify();
                                        },
                                    )),
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
                                    .tooltip(
                                        "Toggle the camera inspector",
                                    )
                                    .on_click(cx.listener(
                                        |view, _, _, cx| {
                                            view.inspector =
                                                !view.inspector;
                                            cx.notify();
                                        },
                                    )),
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
                        .children(chooser),
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
                                    .when(
                                        visible.is_empty(),
                                        |view| {
                                            view.child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .items_center()
                                                    .justify_center()
                                                    .gap_2()
                                                    .p_8()
                                                    .text_color(
                                                        cx.theme().muted_foreground,
                                                    )
                                                    .child(
                                                        "No cameras configured",
                                                    )
                                                    .child(
                                                        "Open Cameras to add an authorized RTSP stream.",
                                                    ),
                                            )
                                        },
                                    )
                                    .children(rows),
                            ),
                    )
                    .when(docked, |body| {
                        body.child(
                            div()
                                .w(px(300.))
                                .flex_shrink_0()
                                .id("live-inspector-scroll")
                                .overflow_y_scroll()
                                .children(
                                    chosen.as_ref().map(
                                        |camera| {
                                            self.inspector(
                                                camera, cx,
                                            )
                                        },
                                    ),
                                ),
                        )
                    }),
            )
            .when(
                self.inspector
                    && !docked
                    && chosen.is_some(),
                |root| {
                    root.child(
                        div()
                            .px_3()
                            .pb_3()
                            .max_h(px(220.))
                            .id("compact-inspector-scroll")
                            .overflow_y_scroll()
                            .children(
                                chosen.as_ref().map(
                                    |camera| {
                                        self.inspector(
                                            camera, cx,
                                        )
                                    },
                                ),
                            ),
                    )
                },
            )
    }
}
