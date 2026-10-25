use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::*;

pub struct RecordingsView;

impl Render for RecordingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_3()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Recordings"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        Icon::new(IconName::HardDrive)
                            .large()
                            .text_color(
                                cx.theme().muted_foreground,
                            ),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                "Recording not available yet",
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(
                                cx.theme().muted_foreground,
                            )
                            .child(
                                "Live viewing does not save camera video or capture anything in the background.",
                            ),
                    ),
            )
    }
}
