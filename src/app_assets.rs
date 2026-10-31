use gpui_kit::*;
use std::borrow::Cow;

pub(crate) const APP_ICON_PATH: &str = "../assets/images/Vermouth.png";
pub(crate) const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/images/Vermouth.png");

#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON_PATH {
            return Ok(Some(Cow::Borrowed(APP_ICON_BYTES)));
        }

        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;

        if APP_ICON_PATH.starts_with(path) {
            paths.push(APP_ICON_PATH.into());
        }

        paths.sort();
        paths.dedup();

        Ok(paths)
    }
}
