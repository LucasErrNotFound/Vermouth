use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;

pub struct DiagnosticsView;

impl Render for DiagnosticsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Diagnostics"),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "RTSP connection status and sanitized errors appear on camera tiles and in the camera list.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .p_4()
                    .child(
                        Icon::new(IconName::Activity),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(
                                        FontWeight::MEDIUM,
                                    )
                                    .child("Media backend"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(
                                        cx.theme().muted_foreground,
                                    )
                                    .child(
                                        "FFmpeg · RTSP over TCP · JPEG frames · video only",
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "No authenticated URLs, video frames, or device identifiers are written to diagnostic logs.",
                    ),
            )
    }
}
