#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClass {
    Narrow,
    Compact,
    Regular,
    Expansive,
}

pub fn window_class(logical_width: f32) -> WindowClass {
    match logical_width {
        w if w < 760.0 => WindowClass::Narrow,
        w if w < 1100.0 => WindowClass::Compact,
        w if w < 1500.0 => WindowClass::Regular,
        _ => WindowClass::Expansive,
    }
}

pub fn grid_columns(available_width: f32, camera_count: usize, focus_mode: bool) -> usize {
    if camera_count == 0 || focus_mode {
        return 1;
    }
    let min_tile_width = 280.0;
    let gap = 12.0;
    let fit = ((available_width + gap) / (min_tile_width + gap))
        .floor()
        .max(1.0) as usize;
    fit.min(camera_count)
        .min(if camera_count == 4 { 2 } else { 3 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn width_classes() {
        assert_eq!(window_class(640.), WindowClass::Narrow);
        assert_eq!(window_class(900.), WindowClass::Compact);
        assert_eq!(window_class(1280.), WindowClass::Regular);
        assert_eq!(window_class(1920.), WindowClass::Expansive);
    }
    #[test]
    fn grid_reflows_without_tiny_tiles() {
        assert_eq!(grid_columns(350., 4, false), 1);
        assert_eq!(grid_columns(640., 4, false), 2);
        assert_eq!(grid_columns(960., 4, false), 2);
        assert_eq!(grid_columns(960., 5, false), 3);
        assert_eq!(grid_columns(960., 4, true), 1);
    }
}
