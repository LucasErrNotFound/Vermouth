mod discovery;
mod rtsp_candidates;

use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogEntrance,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    spinner::Spinner,
};
use gpui_kit::*;

use crate::camera::{Camera, CameraCatalog, CameraId, ConnectionState};

use discovery::{DiscoveredCamera, configured_camera_ip};
use rtsp_candidates::{Candidate, ProbeResult};

type ScanResult = Result<Vec<DiscoveredCamera>, String>;

pub struct CamerasView {
    catalog: Entity<CameraCatalog>,
    search: Entity<InputState>,
    discovered: Vec<DiscoveredCamera>,
    scanning: bool,
    scan_complete: bool,
    scan_message: Option<String>,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_tx: Sender<ScanResult>,
    scan_rx: Receiver<ScanResult>,
    _subscriptions: Vec<Subscription>,
    _scan_task: Task<()>,
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

        let (scan_tx, scan_rx) = mpsc::channel();

        let scan_task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;

                if weak
                    .update_in(cx, |view, _, cx| {
                        view.poll_scan(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            catalog,
            search,
            discovered: Vec::new(),
            scanning: false,
            scan_complete: false,
            scan_message: None,
            scan_cancel: None,
            scan_tx,
            scan_rx,
            _subscriptions: vec![observer, subscription],
            _scan_task: scan_task,
        }
    }

    fn start_scan(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            if let Some(cancel) = &self.scan_cancel {
                cancel.store(true, Ordering::Relaxed);

                self.scan_message = Some("Cancelling the current network scan...".into());

                cx.notify();
            }

            return;
        }

        self.discovered.clear();
        self.scanning = true;
        self.scan_complete = false;
        self.scan_message = None;

        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        let tx = self.scan_tx.clone();

        self.scan_cancel = Some(cancelled);

        thread::spawn(move || {
            let result = discovery::scan(worker_cancel);
            let _ = tx.send(result);
        });

        cx.notify();
    }

    fn poll_scan(&mut self, cx: &mut Context<Self>) {
        while let Ok(outcome) = self.scan_rx.try_recv() {
            self.scanning = false;
            self.scan_complete = true;
            self.scan_cancel = None;

            match outcome {
                Ok(cameras) => {
                    self.discovered = cameras;
                    self.scan_message = None;
                }
                Err(message) => {
                    self.discovered.clear();
                    self.scan_message = Some(message);
                }
            }

            cx.notify();
        }
    }

    pub(crate) fn open_editor(
        &mut self,
        editing: Option<CameraId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_editor(editing, None, window, cx);
    }

    fn open_discovered(
        &mut self,
        discovered: DiscoveredCamera,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_editor(None, Some(discovered), window, cx);
    }

    fn show_editor(
        &mut self,
        editing: Option<CameraId>,
        discovered: Option<DiscoveredCamera>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalog = self.catalog.clone();

        let editor = cx.new(|cx| CameraForm::new(catalog, editing, discovered, window, cx));

        window.open_dialog(cx, move |dialog, _, _| {
            let submit = editor.clone();

            dialog
                .title(if editing.is_some() {
                    "Edit camera"
                } else {
                    "Add RTSP camera"
                })
                .entrance(DialogEntrance::SlideDown)
                .overlay_closable(false)
                .on_ok(move |_, window, cx| {
                    submit.update(cx, |form, cx| {
                        form.save(window, cx);
                    });

                    false
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

    pub(crate) fn confirm_remove(
        &mut self,
        id: CameraId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                .entrance(DialogEntrance::SlideDown)
                .on_ok(|_, _, _| false)
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

                                    if !removed {
                                        let error = catalog.read(cx)
                                            .storage_error()
                                            .map(str::to_owned);
                                        if let Some(error) = error {
                                            window.push_notification(
                                                Notification::error(error)
                                                    .autohide(false)
                                                    .placement(Anchor::BottomRight),
                                                cx,
                                            );
                                        }
                                    }

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

    fn discovered_row(&mut self, device: DiscoveredCamera, cx: &mut Context<Self>) -> AnyElement {
        let address = device.address.to_string();
        let description = device.service_summary();

        let accent = if device.onvif_advertised {
            cx.theme().success
        } else if device.v380_advertised {
            cx.theme().info
        } else {
            cx.theme().warning
        };

        let label = if device.onvif_advertised {
            "ONVIF device discovered"
        } else if device.v380_advertised {
            "V380 device discovered"
        } else {
            "Possible camera or network service"
        };

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
                    .items_center()
                    .justify_center()
                    .w(px(36.))
                    .h(px(36.))
                    .rounded_lg()
                    .bg(cx.theme().secondary)
                    .child(Icon::new(IconName::Camera).text_color(accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(address),
                            )
                            .child(div().text_xs().text_color(accent).child(label)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    ),
            )
            .child(
                Button::new(format!("configure-discovered-{}", device.address))
                    .icon(IconName::Plus)
                    .label("Configure")
                    .cursor_pointer()
                    .tooltip("Configure an authorized RTSP stream for this device")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_discovered(device.clone(), window, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Drop for CamerasView {
    fn drop(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Render for CamerasView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();

        let configured = self.catalog.read(cx).all().to_vec();
        let storage_error = self.catalog.read(cx).storage_error().map(str::to_owned);

        let configured_addresses = configured
            .iter()
            .filter_map(|camera| configured_camera_ip(&camera.rtsp_address))
            .collect::<HashSet<_>>();

        let added = configured
            .into_iter()
            .filter(|camera| {
                camera.name.to_lowercase().contains(&query)
                    || camera.rtsp_address.to_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();

        let discovered = self
            .discovered
            .iter()
            .filter(|device| {
                !configured_addresses.contains(&device.address)
                    && (query.is_empty()
                        || device.address.to_string().contains(&query)
                        || device.service_summary().to_lowercase().contains(&query))
            })
            .cloned()
            .collect::<Vec<_>>();

        let added_count = added.len();
        let discovered_count = discovered.len();

        let added_rows = added
            .into_iter()
            .map(|camera| self.camera_row(camera, cx))
            .collect::<Vec<_>>();

        let discovered_rows = discovered
            .into_iter()
            .map(|device| self.discovered_row(device, cx))
            .collect::<Vec<_>>();

        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

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
                    .gap_3()
                    .px_5()
                    .py_4()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(
                                        FontWeight::SEMIBOLD,
                                    )
                                    .child("Cameras"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(
                                        "Manage configured cameras and discover local devices",
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("scan-cameras")
                                    .outline()
                                    .icon(if self.scanning {
                                        IconName::X
                                    } else {
                                        IconName::Search
                                    })
                                    .label(if self.scanning {
                                        "Cancel scan"
                                    } else {
                                        "Scan for Cameras"
                                    })
                                    .cursor_pointer()
                                    .tooltip(if self.scanning {
                                        "Cancel the current LAN scan"
                                    } else {
                                        "Search private local networks for camera services"
                                    })
                                    .on_click(cx.listener(
                                        |view, _, _, cx| {
                                            view.start_scan(cx);
                                        },
                                    )),
                            )
                            .child(
                                Button::new("add-camera")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("Add Camera")
                                    .cursor_pointer()
                                    .tooltip(
                                        "Manually add an authorized RTSP source",
                                    )
                                    .on_click(cx.listener(
                                        |view, _, window, cx| {
                                            view.open_editor(
                                                None,
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .p_4()
                    .when_some(storage_error, |view, error| {
                        view.child(
                            div()
                                .p_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(cx.theme().danger)
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(format!("Camera storage: {error}")),
                        )
                    })
                    .child(
                        Input::new(&self.search)
                            .cleanable(true),
                    )
                    .child(
                        div()
                            .id("camera-list-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Icon::new(IconName::Video)
                                                    .text_color(muted),
                                            )
                                            .child(
                                                div()
                                                    .font_weight(
                                                        FontWeight::SEMIBOLD,
                                                    )
                                                    .child(format!(
                                                        "Added Cameras ({added_count})"
                                                    )),
                                            ),
                                    )
                                    .when(
                                        added_count == 0,
                                        |section| {
                                            section.child(
                                                div()
                                                    .p_5()
                                                    .rounded_lg()
                                                    .border_1()
                                                    .border_color(border)
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child(if query.is_empty() {
                                                        "No cameras added yet. Add a camera manually or scan your network."
                                                    } else {
                                                        "No added cameras match your search."
                                                    }),
                                            )
                                        },
                                    )
                                    .children(added_rows)
                                    .child(
                                        div()
                                            .border_t_1()
                                            .border_color(border)
                                            .pt_4()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        Icon::new(
                                                            IconName::Search,
                                                        )
                                                        .text_color(muted),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_weight(
                                                                FontWeight::SEMIBOLD,
                                                            )
                                                            .child(format!(
                                                                "Discovered Cameras ({discovered_count})"
                                                            )),
                                                    ),
                                            )
                                            .when(
                                                self.scanning,
                                                |heading| {
                                                    heading.child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .text_sm()
                                                            .text_color(
                                                                cx.theme().info,
                                                            )
                                                            .child(
                                                                Spinner::new()
                                                                    .small()
                                                                    .color(
                                                                        cx.theme().info,
                                                                    ),
                                                            )
                                                            .child("Scanning LAN"),
                                                    )
                                                },
                                            ),
                                    )
                                    .when_some(
                                        self.scan_message.clone(),
                                        |section, message| {
                                            section.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(
                                                        cx.theme().warning,
                                                    )
                                                    .child(message),
                                            )
                                        },
                                    )
                                    .when(
                                        discovered_count == 0,
                                        |section| {
                                            section.child(
                                                div()
                                                    .p_5()
                                                    .rounded_lg()
                                                    .border_1()
                                                    .border_color(border)
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child(
                                                        if self.scanning {
                                                            "Searching local network interfaces..."
                                                        } else if !self.scan_complete {
                                                            "Select Scan for Cameras to find devices on your local network."
                                                        } else if !query.is_empty() {
                                                            "No discovered devices match your search."
                                                        } else {
                                                            "No additional camera candidates were found."
                                                        },
                                                    ),
                                            )
                                        },
                                    )
                                    .children(discovered_rows)
                                    .child(
                                        div()
                                            .pt_2()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(
                                                "Discovery is limited to private local networks. An open port does not guarantee camera compatibility, RTSP access, or ONVIF support.",
                                            ),
                                    ),
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
    hint: Option<String>,
    error: Option<String>,
    suggested_address: String,
    probe_rx: Option<Receiver<Vec<Candidate>>>,
    probe_cancel: Option<Arc<AtomicBool>>,
    _probe_task: Option<Task<()>>,
}

impl CameraForm {
    fn new(
        catalog: Entity<CameraCatalog>,
        editing: Option<CameraId>,
        discovered: Option<DiscoveredCamera>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = editing.and_then(|id| catalog.read(cx).get(id).cloned());

        let suggested_name = discovered
            .as_ref()
            .map(|device| format!("Camera {}", device.address))
            .unwrap_or_default();

        let suggested_address = discovered
            .as_ref()
            .map(|device| {
                let port = device.rtsp_port().unwrap_or(554);
                if device.v380_advertised {
                    format!("rtsp://{}:{port}/live/ch00_1", device.address)
                } else {
                    format!("rtsp://{}:{port}/", device.address)
                }
            })
            .unwrap_or_default();

        let hint = discovered.as_ref().map(|device| {
            if device.v380_advertised {
                "Testing documented V380 main and substream paths. The suggested address is not yet verified."
                    .to_owned()
            } else {
                "Discovery does not reveal an RTSP path. Enter the camera's verified stream URL. ONVIF GetStreamUri is not yet supported."
                    .to_owned()
            }
        });

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Front entrance")
                .default_value(
                    initial
                        .as_ref()
                        .map(|camera| camera.name.as_str())
                        .unwrap_or(&suggested_name),
                )
        });

        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("rtsp://192.168.1.20:554/live")
                .default_value(
                    initial
                        .as_ref()
                        .map(|camera| camera.rtsp_address.as_str())
                        .unwrap_or(&suggested_address),
                )
        });

        let mut probe_rx = None;
        let mut probe_cancel = None;
        let mut probe_task = None;

        if let Some(device) = discovered.filter(|device| device.v380_advertised) {
            let (tx, rx) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let ip = device.address;
            let ports = device.open_ports;

            thread::spawn(move || {
                let result = rtsp_candidates::probe_v380(ip, &ports, &worker_cancel);
                let _ = tx.send(result);
            });

            let task = cx.spawn_in(window, async move |weak, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;

                    match weak.update_in(cx, |form, window, cx| form.poll_probe(window, cx)) {
                        Ok(true) => break,
                        Ok(false) => (),
                        Err(_) => break,
                    }
                }
            });

            probe_rx = Some(rx);
            probe_cancel = Some(cancel);
            probe_task = Some(task);
        }

        Self {
            catalog,
            editing,
            name,
            address,
            hint,
            error: None,
            suggested_address,
            probe_rx,
            probe_cancel,
            _probe_task: probe_task,
        }
    }

    fn poll_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(rx) = self.probe_rx.as_ref() else {
            return true;
        };

        let candidates = match rx.try_recv() {
            Ok(candidates) => candidates,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.hint = Some(
                    "The RTSP check could not finish. Confirm the stream URL manually.".into(),
                );
                self.probe_rx = None;
                cx.notify();
                return true;
            }
        };

        if let Some(candidate) = rtsp_candidates::preferred(&candidates) {
            let unchanged =
                self.address.read(cx).value().as_ref() == self.suggested_address.as_str();
            if unchanged {
                self.address.update(cx, |field, cx| {
                    field.set_value(candidate.url.clone(), window, cx);
                });
            }

            self.hint = Some(match candidate.result {
                ProbeResult::VideoSdpAccepted => {
                    "The camera accepted RTSP DESCRIBE and advertised video. Actual playback will be checked when connected."
                        .to_owned()
                }
                ProbeResult::AuthenticationRequired => {
                    "RTSP authentication is required. Credential storage is not yet supported; supply an authorized credential-free stream."
                        .to_owned()
                }
                ProbeResult::Rejected | ProbeResult::Indeterminate => {
                    "No stream URL could be verified. The suggested V380 path may not work; confirm the camera's stream settings."
                        .to_owned()
                }
            });
        } else {
            self.hint = Some("No V380 stream path responded. Enter the URL manually.".into());
        }

        self.probe_rx = None;
        cx.notify();
        true
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
                        format!("Saved '{name}'. Connecting to stream...")
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

impl Drop for CameraForm {
    fn drop(&mut self) {
        if let Some(cancel) = &self.probe_cancel {
            cancel.store(true, Ordering::Relaxed);
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
                    .text_color(
                        cx.theme().muted_foreground,
                    )
                    .child(
                        "Only add cameras you are authorized to view. FFmpeg must be installed. Credentials and query tokens are not supported in this release.",
                    ),
            )
            .when_some(
                self.hint.clone(),
                |form, hint| {
                    form.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().warning)
                            .child(hint),
                    )
                },
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
