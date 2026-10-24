use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;

use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    notification::Notification,
};
use gpui_kit::*;

use crate::camera::{Camera, CameraCatalog, CameraId, CameraTransport};

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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search demo cameras"));
        let observer = cx.observe(&catalog, |_, _, cx| cx.notify());
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
                    "Edit demo camera"
                } else {
                    "Add demo camera"
                })
                .child(editor.clone())
        });
    }

    fn confirm_remove(&mut self, id: CameraId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .catalog
            .read(cx)
            .get(id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let catalog = self.catalog.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let catalog = catalog.clone();
            let name_for_notice = name.clone();
            dialog.title("Remove demo camera?")
                .child(format!("Remove '{name}' from this in-memory workspace? This does not change any physical camera."))
                .footer(div().flex().justify_end().gap_2()
                    .child(Button::new("cancel-removal").label("Cancel")
                        .cursor_pointer().tooltip("Keep this camera in the demo workspace")
                        .on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("confirm-removal").danger().icon(IconName::Trash).label("Remove")
                        .cursor_pointer().tooltip("Remove the saved demo camera, not the physical device")
                        .on_click(move |_, window, cx| {
                            let removed = catalog.update(cx, |store, cx| {
                                let removed = store.remove(id);
                                if removed { cx.notify(); }
                                removed
                            });
                            window.close_dialog(cx);
                            if removed {
                                window.push_notification(Notification::success(
                                    format!("Removed demo camera '{name_for_notice}'."))
                                    .placement(Anchor::BottomRight), cx);
                            }
                        })))
        });
    }

    fn camera_row(&mut self, camera: Camera, cx: &mut Context<Self>) -> AnyElement {
        let id = camera.id;
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
                            .truncate()
                            .child(format!(
                                "{} · {}",
                                camera.transport.label(),
                                camera.connection.label()
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(camera.rtsp_address),
                    ),
            )
            .child(
                Button::new(format!("edit-{}", id.0))
                    .icon(IconName::Pencil)
                    .label("Edit")
                    .cursor_pointer()
                    .tooltip("Edit this demo camera's name and RTSP address")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_editor(Some(id), window, cx)
                    })),
            )
            .child(
                Button::new(format!("remove-{}", id.0))
                    .icon(IconName::Trash)
                    .label("Remove")
                    .danger()
                    .outline()
                    .cursor_pointer()
                    .tooltip("Remove this camera from the demo workspace")
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.confirm_remove(id, window, cx)),
                    ),
            )
            .into_any_element()
    }
}

impl Render for CamerasView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let all = self.catalog.read(cx).all().to_vec();
        let filtered = all
            .into_iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&query)
                    || c.transport.label().to_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();
        let is_empty = filtered.is_empty();
        let rows = filtered
            .into_iter()
            .map(|camera| self.camera_row(camera, cx))
            .collect::<Vec<_>>();

        div().flex().flex_col().size_full().min_h_0().min_w_0()
            .bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().flex().items_center().justify_between().gap_2().px_5().py_4()
                .border_b_1().border_color(cx.theme().border)
                .child(div().flex().flex_col().gap_1().min_w_0()
                    .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child("Cameras"))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground)
                        .child("Manage simulated camera configurations")))
                .child(Button::new("add-camera").primary().icon(IconName::Plus).label("Add camera")
                    .cursor_pointer().tooltip("Add a simulated camera configuration")
                    .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx)))))
            .child(div().flex().flex_col().min_h_0().flex_1().p_4().gap_3()
                .child(Input::new(&self.search).cleanable(true))
                .child(div().id("camera-list-scroll").min_h_0().flex_1().overflow_y_scroll()
                    .child(div().flex().flex_col().gap_2()
                        .when(is_empty, |list| list.child(div().p_8()
                            .text_color(cx.theme().muted_foreground)
                            .child(if query.is_empty() { "No cameras configured. Add a demo camera to begin." }
                                    else { "No cameras match this search." })))
                        .children(rows))))
    }
}

struct CameraForm {
    catalog: Entity<CameraCatalog>,
    editing: Option<CameraId>,
    name: Entity<InputState>,
    address: Entity<InputState>,
    onvif: bool,
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
                .placeholder("e.g. Office camera")
                .default_value(initial.as_ref().map(|c| c.name.as_str()).unwrap_or(""))
        });
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("rtsp://demo.invalid/office")
                .default_value(
                    initial
                        .as_ref()
                        .map(|c| c.rtsp_address.as_str())
                        .unwrap_or(""),
                )
        });
        let onvif = initial
            .as_ref()
            .is_some_and(|c| c.transport == CameraTransport::OnvifRtsp);
        Self {
            catalog,
            editing,
            name,
            address,
            onvif,
            error: None,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        let address = self.address.read(cx).value().to_string();
        let editing = self.editing;
        let onvif = self.onvif;
        let result = self.catalog.update(cx, |store, cx| {
            let result = if let Some(id) = editing {
                store.update(id, &name, &address, onvif)
            } else {
                store.add(&name, &address, onvif).map(|_| ())
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
                    Notification::success(if editing.is_some() {
                        format!("Updated demo camera '{name}'.")
                    } else {
                        format!("Added demo camera '{name}'.")
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
        div().flex().flex_col().gap_3().w_full()
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Demo only: configurations stay in memory; no network connection will be made."))
            .child(div().flex().flex_col().gap_1()
                .child(div().text_sm().child("Camera name"))
                .child(Input::new(&self.name)))
            .child(div().flex().flex_col().gap_1()
                .child(div().text_sm().child("RTSP address (without credentials)"))
                .child(Input::new(&self.address)))
            .child(div().flex().gap_2()
                .child(Button::new("transport-rtsp").label(if self.onvif { "RTSP only" } else { "✓ RTSP only" })
                    .cursor_pointer().tooltip("Use a manual RTSP stream without ONVIF controls")
                    .on_click(cx.listener(|this, _, _, cx| { this.onvif = false; cx.notify(); })))
                .child(Button::new("transport-onvif").label(if self.onvif { "✓ ONVIF-capable" } else { "ONVIF-capable" })
                    .cursor_pointer().tooltip("Mark this demo configuration as ONVIF-capable")
                    .on_click(cx.listener(|this, _, _, cx| { this.onvif = true; cx.notify(); }))))
            .when_some(self.error.clone(), |form, error| form.child(
                div().text_sm().text_color(cx.theme().danger).child(error)))
            .child(div().flex().justify_end().gap_2().pt_2()
                .child(Button::new("cancel-editor").label("Cancel")
                    .cursor_pointer().tooltip("Discard changes and close the dialog")
                    .on_click(|_, window, cx| window.close_dialog(cx)))
                .child(Button::new("save-editor").primary().icon(IconName::Check).label(if self.editing.is_some() { "Save changes" } else { "Add camera" })
                    .cursor_pointer().tooltip("Validate and save the demo configuration in memory")
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))))
    }
}
