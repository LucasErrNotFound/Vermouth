use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::assets::IconName;
use gpui_kit::base::{Spring, spring};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::Button,
    menu::{ContextMenuExt as _, PopupMenuItem},
    notification::Notification,
    spinner::Spinner,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub mod layout;

use crate::{
    camera::{Camera, CameraCatalog, CameraId, ConnectionState},
    features::cameras::CamerasView,
};

use layout::{WindowClass, grid_columns, window_class};

const GRID_GAP: f32 = 12.0;
const HEADER_HEIGHT: f32 = 40.0;
const FOOTER_HEIGHT: f32 = 40.0;

struct ReconnectNotice;

#[derive(Clone)]
struct CameraDrag {
    id: CameraId,
}

impl Render for CameraDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.)).opacity(0.)
    }
}

pub struct LiveView {
    catalog: Entity<CameraCatalog>,
    cameras_view: Entity<CamerasView>,
    selected: Option<CameraId>,
    focused: bool,
    inspector: bool,
    navigation_collapsed: Option<bool>,
    camera_order: Vec<CameraId>,
    frames: HashMap<CameraId, (u64, Arc<RenderImage>)>,
    retired: VecDeque<(Instant, Arc<RenderImage>)>,
    pending_connections: HashMap<CameraId, String>,
    _observer: Subscription,
    _poll_task: Task<()>,
}

impl LiveView {
    pub fn new(
        catalog: Entity<CameraCatalog>,
        cameras_view: Entity<CamerasView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let camera_order = catalog
            .read(cx)
            .all()
            .iter()
            .map(|camera| camera.id)
            .collect();

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

        let task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(67))
                    .await;

                if weak
                    .update_in(cx, |view, window, cx| {
                        view.tick(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            catalog,
            cameras_view,
            selected: None,
            focused: false,
            inspector: true,
            navigation_collapsed: None,
            camera_order,
            frames: HashMap::new(),
            retired: VecDeque::new(),
            pending_connections: HashMap::new(),
            _observer: observer,
            _poll_task: task,
        }
    }

    fn sync_camera_order(&mut self, cameras: &[Camera]) {
        let ids: HashSet<_> = cameras.iter().map(|camera| camera.id).collect();

        self.camera_order.retain(|id| ids.contains(id));

        for camera in cameras {
            if !self.camera_order.contains(&camera.id) {
                self.camera_order.push(camera.id);
            }
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.catalog.update(cx, |catalog, cx| {
            if catalog.poll() {
                cx.notify();
            }
        });

        let cameras = self.catalog.read(cx).all().to_vec();
        let now = Instant::now();

        self.sync_camera_order(&cameras);

        let mut changed = false;

        let removed: Vec<_> = self
            .frames
            .keys()
            .copied()
            .filter(|id| !cameras.iter().any(|camera| camera.id == *id))
            .collect();

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

            if self
                .frames
                .get(&camera.id)
                .is_some_and(|(revision, _)| *revision == camera.frame_revision)
            {
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
                cx.drop_image(image, Some(window));
            }
        }

        let completed: Vec<_> = self
            .pending_connections
            .iter()
            .filter_map(|(&id, name)| {
                let camera = cameras.iter().find(|camera| camera.id == id);

                match camera {
                    Some(camera) if camera.connection == ConnectionState::Live => {
                        Some((id, name.clone(), true, None))
                    }
                    Some(camera) if camera.connection == ConnectionState::Error => {
                        Some((id, name.clone(), false, camera.detail))
                    }
                    None => Some((id, name.clone(), false, None)),
                    _ => None,
                }
            })
            .collect();

        for (id, name, success, reason) in completed {
            self.pending_connections.remove(&id);

            let key = id.0.to_string();

            if success {
                window.push_notification(
                    Notification::success(format!("{name} is receiving live video."))
                        .id1::<ReconnectNotice>(key)
                        .title("Camera connected")
                        .placement(Anchor::BottomRight),
                    cx,
                );
            } else {
                window.push_notification(
                    Notification::error(reason.unwrap_or("The camera could not be connected."))
                        .id1::<ReconnectNotice>(key)
                        .title(format!("{name} — connection failed"))
                        .placement(Anchor::BottomRight),
                    cx,
                );
            }
        }

        if changed {
            cx.notify();
        }
    }

    fn start_connection(&mut self, id: CameraId, window: &mut Window, cx: &mut Context<Self>) {
        let name = match self.catalog.read(cx).get(id) {
            Some(camera) => camera.name.clone(),
            None => return,
        };

        let result = self.catalog.update(cx, |catalog, cx| {
            let result = catalog.connect(id);

            if result.is_ok() {
                cx.notify();
            }

            result
        });

        match result {
            Ok(()) => {
                self.pending_connections.insert(id, name.clone());

                window.push_notification(
                    Notification::info("Opening the RTSP stream...")
                        .id1::<ReconnectNotice>(id.0.to_string())
                        .title(format!("Connecting — {name}"))
                        .autohide(false)
                        .placement(Anchor::BottomRight)
                        .content(|_, _, cx| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Spinner::new().small().color(cx.theme().info))
                                .child("Waiting for video frames")
                                .into_any_element()
                        }),
                    cx,
                );
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error).placement(Anchor::BottomRight),
                    cx,
                );
            }
        }
    }

    fn disconnect_camera(&mut self, id: CameraId, window: &mut Window, cx: &mut Context<Self>) {
        let name = match self.catalog.read(cx).get(id) {
            Some(camera) => camera.name.clone(),
            None => return,
        };

        self.catalog.update(cx, |catalog, cx| {
            catalog.disconnect(id);
            cx.notify();
        });

        self.pending_connections.remove(&id);

        window.push_notification(
            Notification::info(format!("{name} disconnected."))
                .id1::<ReconnectNotice>(id.0.to_string())
                .placement(Anchor::BottomRight),
            cx,
        );
    }

    fn move_camera(&mut self, dragged: CameraId, destination: usize, cx: &mut Context<Self>) {
        let Some(source) = self.camera_order.iter().position(|id| *id == dragged) else {
            return;
        };

        if source == destination || destination >= self.camera_order.len() {
            return;
        }

        self.camera_order.remove(source);
        self.camera_order.insert(destination, dragged);

        cx.notify();
    }

    fn drag_move(
        &mut self,
        dragged: CameraId,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        columns: usize,
        tile_size: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if columns == 0 || self.camera_order.len() < 2 {
            return;
        }

        let (tile_width, tile_height) = tile_size;

        let x = (position.x - bounds.origin.x).as_f32();
        let y = (position.y - bounds.origin.y).as_f32();

        if x < 0.0 || y < 0.0 || x >= bounds.size.width.as_f32() || y >= bounds.size.height.as_f32()
        {
            return;
        }

        let column = ((x / (tile_width + GRID_GAP)) as usize).min(columns - 1);

        let row = (y / (tile_height + GRID_GAP)) as usize;

        let destination = (row * columns + column).min(self.camera_order.len() - 1);

        self.move_camera(dragged, destination, cx);
    }

    pub(crate) fn set_navigation_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.navigation_collapsed = Some(collapsed);
        cx.notify();
    }

    fn select(&mut self, id: CameraId, cx: &mut Context<Self>) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            cx.notify();
        }
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

    fn tile(
        &mut self,
        camera: Camera,
        selected: bool,
        frame_height: f32,
        allow_drag: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = camera.id;
        let name = camera.name.clone();

        let frame = self.frames.get(&id).map(|(_, frame)| frame.clone());

        let border = if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        };

        let active = matches!(
            camera.connection,
            ConnectionState::Connecting | ConnectionState::Live | ConnectionState::Stale
        );

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
                    ConnectionState::Connecting => "Connecting...",
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

        let weak_drag = cx.weak_entity();
        let weak_connect = cx.weak_entity();
        let weak_disconnect = cx.weak_entity();
        let weak_edit = self.cameras_view.downgrade();
        let weak_delete = self.cameras_view.downgrade();

        let success = cx.theme().success;
        let info = cx.theme().info;
        let warning = cx.theme().warning;
        let danger = cx.theme().danger;
        let foreground = cx.theme().foreground;

        let context_menu = div()
            .id(format!("camera-content-{}", id.0))
            .flex()
            .flex_col()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, _, _, cx| {
                    view.select(id, cx);
                }),
            )
            .context_menu(move |menu, _, _| {
                let connect = weak_connect.clone();
                let disconnect = weak_disconnect.clone();
                let edit = weak_edit.clone();
                let delete = weak_delete.clone();

                let menu = if active {
                    menu.item(menu_action_item(
                        "Reconnect",
                        IconName::RefreshCw,
                        info,
                        move |_, window, cx| {
                            let _ = connect.update(cx, |view, cx| {
                                view.start_connection(id, window, cx);
                            });
                        },
                    ))
                } else {
                    menu.item(menu_action_item(
                        "Connect",
                        IconName::Wifi,
                        success,
                        move |_, window, cx| {
                            let _ = connect.update(cx, |view, cx| {
                                view.start_connection(id, window, cx);
                            });
                        },
                    ))
                };

                let menu = if active {
                    menu.item(menu_action_item(
                        "Disconnect",
                        IconName::WifiOff,
                        warning,
                        move |_, window, cx| {
                            let _ = disconnect.update(cx, |view, cx| {
                                view.disconnect_camera(id, window, cx);
                            });
                        },
                    ))
                } else {
                    menu
                };

                menu.separator()
                    .item(menu_action_item(
                        "Edit camera",
                        IconName::Pencil,
                        foreground,
                        move |_, window, cx| {
                            let _ = edit.update(cx, |view, cx| {
                                view.open_editor(Some(id), window, cx);
                            });
                        },
                    ))
                    .item(menu_action_item(
                        "Delete camera",
                        IconName::Trash,
                        danger,
                        move |_, window, cx| {
                            let _ = delete.update(cx, |view, cx| {
                                view.confirm_remove(id, window, cx);
                            });
                        },
                    ))
            })
            .child(
                div()
                    .w_full()
                    .h(px(frame_height))
                    .overflow_hidden()
                    .bg(rgb(0x11171e))
                    .text_color(rgb(0xc1ccd7))
                    .child(middle),
            )
            .child(
                div()
                    .h(px(FOOTER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
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
            );

        let drag_payload = CameraDrag { id };

        let tip = format!("Select {}", camera.name);

        let header = div()
            .id(format!("camera-drag-header-{}", id.0))
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .when(allow_drag, |header| {
                header.child(
                    Icon::new(IconName::GripVertical)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_sm()
                    .child(name),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if selected { "SELECTED" } else { "RTSP" }),
            );

        let header = if allow_drag {
            header
                .cursor(CursorStyle::OpenHand)
                .active(|style| style.cursor(CursorStyle::ClosedHand))
                .tooltip(|window, cx| Tooltip::new("Drag to rearrange cameras").build(window, cx))
                .on_drag(drag_payload, move |drag, _, _, cx| {
                    let _ = weak_drag.update(cx, |view, cx| {
                        view.select(drag.id, cx);
                    });

                    cx.new(|_| drag.clone())
                })
        } else {
            header
        };

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
            .child(header)
            .child(context_menu)
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

fn menu_action_item(
    label: &'static str,
    icon: IconName,
    color: Hsla,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(Icon::new(icon).small().text_color(color))
            .child(div().text_color(color).child(label))
    })
    .on_click(handler)
}

impl Render for LiveView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width.as_f32();
        let narrow = window_class(width) == WindowClass::Narrow;

        let docked = width >= 1500.0 && self.inspector;

        let cameras = self.catalog.read(cx).all().to_vec();

        self.sync_camera_order(&cameras);

        let ordered = self
            .camera_order
            .iter()
            .filter_map(|id| cameras.iter().find(|camera| camera.id == *id).cloned())
            .collect::<Vec<_>>();

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
        )
        .max(1);

        let show_chooser = (self.focused || narrow) && cameras.len() > 1;

        let allow_drag = !(self.focused || narrow);

        let chooser = if show_chooser {
            ordered
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
                        .tooltip(format!("Select {}", camera.name,))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select(id, cx);
                        }))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let visible = if self.focused || narrow {
            ordered
                .iter()
                .filter(|camera| Some(camera.id) == selected)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            ordered.clone()
        };

        let grid_columns = if self.focused || narrow { 1 } else { columns };

        let grid_width = (content_width - 36.0).max(280.0);

        let tile_width = (grid_width - GRID_GAP * (grid_columns - 1) as f32) / grid_columns as f32;

        let frame_height = tile_width * 9.0 / 16.0;
        let tile_height = HEADER_HEIGHT + frame_height + FOOTER_HEIGHT;

        let row_count = visible.len().div_ceil(grid_columns);

        let grid_height = if row_count == 0 {
            0.0
        } else {
            row_count as f32 * tile_height + (row_count - 1) as f32 * GRID_GAP
        };

        let mut tiles = Vec::with_capacity(visible.len());

        for (index, camera) in visible.iter().enumerate() {
            let column = index % grid_columns;
            let row = index / grid_columns;

            let target_x = column as f32 * (tile_width + GRID_GAP);
            let target_y = row as f32 * (tile_height + GRID_GAP);

            let x = spring(
                format!("camera-grid-x-{}", camera.id.0),
                target_x,
                Spring::new(Duration::from_millis(240)).with_damping(0.78),
                window,
                cx,
            );

            let y = spring(
                format!("camera-grid-y-{}", camera.id.0),
                target_y,
                Spring::new(Duration::from_millis(240)).with_damping(0.78),
                window,
                cx,
            );

            let card = self.tile(
                camera.clone(),
                Some(camera.id) == selected,
                frame_height,
                allow_drag,
                cx,
            );

            tiles.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(tile_width))
                    .h(px(tile_height))
                    .child(card),
            );
        }

        let camera_grid = div()
            .id("live-draggable-camera-grid")
            .relative()
            .w_full()
            .h(px(grid_height))
            .children(tiles)
            .on_drag_move(
                cx.listener(move |view, event: &DragMoveEvent<CameraDrag>, _, cx| {
                    if !allow_drag || show_chooser {
                        return;
                    }

                    let dragged = event.drag(cx).id;

                    view.drag_move(
                        dragged,
                        event.event.position,
                        event.bounds,
                        grid_columns,
                        (tile_width, tile_height),
                        cx,
                    );
                }),
            )
            .on_drop(cx.listener(|_, _: &CameraDrag, _, cx| {
                cx.notify();
            }));

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
                                        "Drag camera headers to rearrange · Right-click video for actions",
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
                                        "Toggle camera inspector",
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
                                    .w_full()
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
                                    .child(camera_grid),
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
                                    chosen.as_ref().map(|camera| {
                                        self.inspector(
                                            camera, cx,
                                        )
                                    }),
                                ),
                        )
                    }),
            )
            .when(
                self.inspector && !docked && chosen.is_some(),
                |root| {
                    root.child(
                        div()
                            .px_3()
                            .pb_3()
                            .max_h(px(220.))
                            .id("compact-inspector-scroll")
                            .overflow_y_scroll()
                            .children(
                                chosen.as_ref().map(|camera| {
                                    self.inspector(
                                        camera, cx,
                                    )
                                }),
                            ),
                    )
                },
            )
    }
}
