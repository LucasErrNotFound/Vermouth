use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    notification::Notification,
};
use gpui_kit::*;

use crate::camera::{Camera, CameraCatalog, CameraId, ConnectionState};

pub struct CamerasView {
    catalog: Entity<CameraCatalog>,
    search: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl CamerasView {
    pub fn new(
        catalog: Entity<CameraCatalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search cameras"));

        let observer = cx.observe(&catalog, |_, _, cx| {
            cx.notify();
        });

        let subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });

        Self {
            catalog,
            search,
            _subscriptions: vec![observer, subscription],
        }
    }

    fn open_editor(
        &mut self,
        editing: Option<CameraId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalog = self.catalog.clone();

        let editor = cx.new(|cx| CameraForm::new(catalog, editing, window, cx));

        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(if editing.is_some() {
                    "Edit camera"
                } else {
                    "Add RTSP camera"
                })
                .child(editor.clone())
        });
    }

    fn retry(&mut self, id: CameraId, cx: &mut Context<Self>) {
        self.catalog.update(cx, |catalog, cx| {
            if catalog.connect(id).is_ok() {
                cx.notify();
            }
        });
    }

    fn disconnect(&mut self, id: CameraId, cx: &mut Context<Self>) {
        self.catalog.update(cx, |catalog, cx| {
            catalog.disconnect(id);
            cx.notify();
        });
    }

    fn confirm_remove(&mut self, id: CameraId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .catalog
            .read(cx)
            .get(id)
            .map(|camera| camera.name.clone())
            .unwrap_or_default();

        let catalog = self.catalog.clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let catalog = catalog.clone();
            let notice = name.clone();

            dialog
                .title("Remove camera?")
                .child(format!(
                    "Remove '{name}' and disconnect its live stream? No changes will be made to the camera itself."
                ))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-removal")
                                .label("Cancel")
                                .cursor_pointer()
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("confirm-removal")
                                .danger()
                                .icon(IconName::Trash)
                                .label("Remove")
                                .cursor_pointer()
                                .on_click(move |_, window, cx| {
                                    let removed = catalog.update(
                                        cx,
                                        |store, cx| {
                                            let removed =
                                                store.remove(id);

                                            if removed {
                                                cx.notify();
                                            }

                                            removed
                                        },
                                    );

                                    window.close_dialog(cx);

                                    if removed {
                                        window.push_notification(
                                            Notification::success(
                                                format!(
                                                    "Removed '{notice}'."
                                                ),
                                            )
                                            .placement(
                                                Anchor::BottomRight,
                                            ),
                                            cx,
                                        );
                                    }
                                }),
                        ),
                )
        });
    }

    fn camera_row(&mut self, camera: Camera, cx: &mut Context<Self>) -> AnyElement {
        let id = camera.id;

        let active = matches!(
            camera.connection,
            ConnectionState::Connecting | ConnectionState::Live | ConnectionState::Stale
        );

        div()
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(camera.name),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("RTSP · {}", camera.connection.label())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(camera.rtsp_address),
                    )
                    .when_some(camera.detail, |view, detail| {
                        view.child(div().text_xs().text_color(cx.theme().danger).child(detail))
                    }),
            )
            .child(
                Button::new(format!("connection-{}", id.0))
                    .icon(if active {
                        IconName::WifiOff
                    } else {
                        IconName::RefreshCw
                    })
                    .label(if active { "Disconnect" } else { "Connect" })
                    .cursor_pointer()
                    .tooltip(if active {
                        "Disconnect the RTSP stream"
                    } else {
                        "Connect to this RTSP stream"
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if active {
                            view.disconnect(id, cx);
                        } else {
                            view.retry(id, cx);
                        }
                    })),
            )
            .child(
                Button::new(format!("edit-{}", id.0))
                    .icon(IconName::Pencil)
                    .label("Edit")
                    .cursor_pointer()
                    .tooltip("Edit this camera configuration")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_editor(Some(id), window, cx);
                    })),
            )
            .child(
                Button::new(format!("remove-{}", id.0))
                    .icon(IconName::Trash)
                    .label("Remove")
                    .danger()
                    .outline()
                    .cursor_pointer()
                    .tooltip("Remove this camera")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.confirm_remove(id, window, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for CamerasView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();

        let filtered = self
            .catalog
            .read(cx)
            .all()
            .iter()
            .filter(|camera| camera.name.to_lowercase().contains(&query))
            .cloned()
            .collect::<Vec<_>>();

        let empty = filtered.is_empty();

        let rows = filtered
            .into_iter()
            .map(|camera| self.camera_row(camera, cx))
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_5()
                    .py_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Cameras"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Manage authorized RTSP camera connections"),
                            ),
                    )
                    .child(
                        Button::new("add-camera")
                            .primary()
                            .icon(IconName::Plus)
                            .label("Add camera")
                            .cursor_pointer()
                            .tooltip("Add an authorized RTSP source")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_editor(None, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .flex_1()
                    .p_4()
                    .gap_3()
                    .child(Input::new(&self.search).cleanable(true))
                    .child(
                        div()
                            .id("camera-list-scroll")
                            .min_h_0()
                            .flex_1()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .when(empty, |list| {
                                        list.child(
                                            div()
                                                .p_8()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(if query.is_empty() {
                                                    "No cameras added. Select Add camera to begin."
                                                } else {
                                                    "No cameras match this search."
                                                }),
                                        )
                                    })
                                    .children(rows),
                            ),
                    ),
            )
    }
}

struct CameraForm {
    catalog: Entity<CameraCatalog>,
    editing: Option<CameraId>,
    name: Entity<InputState>,
    address: Entity<InputState>,
    error: Option<String>,
}

impl CameraForm {
    fn new(
        catalog: Entity<CameraCatalog>,
        editing: Option<CameraId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = editing.and_then(|id| catalog.read(cx).get(id).cloned());

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Front entrance")
                .default_value(
                    initial
                        .as_ref()
                        .map(|camera| camera.name.as_str())
                        .unwrap_or(""),
                )
        });

        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("rtsp://192.168.1.20:554/live")
                .default_value(
                    initial
                        .as_ref()
                        .map(|camera| camera.rtsp_address.as_str())
                        .unwrap_or(""),
                )
        });

        Self {
            catalog,
            editing,
            name,
            address,
            error: None,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();

        let address = self.address.read(cx).value().to_string();

        let editing = self.editing;

        let result = self.catalog.update(cx, |store, cx| {
            let result = if let Some(id) = editing {
                store.update(id, &name, &address)
            } else {
                store.add(&name, &address).map(|_| ())
            };

            if result.is_ok() {
                cx.notify();
            }

            result
        });

        match result {
            Ok(()) => {
                window.close_dialog(cx);

                window.push_notification(
                    Notification::info(if editing.is_some() {
                        format!("Updated '{name}'. Connecting to stream...")
                    } else {
                        format!("Added '{name}'. Connecting to stream...")
                    })
                    .placement(Anchor::BottomRight),
                    cx,
                );
            }

            Err(message) => {
                self.error = Some(message);
                cx.notify();
            }
        }
    }
}

impl Render for CameraForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Only add cameras you are authorized to view. FFmpeg must be installed. Credentials and query tokens are not supported in this first release.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .child("Camera name"),
                    )
                    .child(Input::new(&self.name)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .child("RTSP address"),
                    )
                    .child(Input::new(&self.address)),
            )
            .when_some(
                self.error.clone(),
                |form, error| {
                    form.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("cancel-editor")
                            .label("Cancel")
                            .cursor_pointer()
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("save-editor")
                            .primary()
                            .icon(IconName::Check)
                            .label(
                                if self.editing.is_some() {
                                    "Save and reconnect"
                                } else {
                                    "Add and connect"
                                },
                            )
                            .cursor_pointer()
                            .on_click(cx.listener(
                                |form, _, window, cx| {
                                    form.save(window, cx);
                                },
                            )),
                    ),
            )
    }
}
