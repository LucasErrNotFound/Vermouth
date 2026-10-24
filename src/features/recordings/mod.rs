use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::*;

pub struct RecordingsView;

impl Render for RecordingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().size_full().gap_3().p_6()
            .bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child("Recordings"))
            .child(div().flex().flex_col().flex_1().items_center().justify_center().gap_3()
                .child(Icon::new(IconName::HardDrive).large()
                    .text_color(cx.theme().muted_foreground))
                .child(div().text_lg().font_weight(FontWeight::MEDIUM).child("No recordings yet"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("Local recording and playback will be available after camera services are implemented."))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("Demo mode never records video or captures anything in the background.")))
    }
}
