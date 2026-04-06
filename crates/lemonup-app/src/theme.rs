use lemonup_core::ThemeMode;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UiTheme {
    pub(crate) brand_hot: Color,
    pub(crate) brand_warm: Color,
    pub(crate) brand_gold: Color,
    pub(crate) warning: Color,
    pub(crate) error: Color,
    pub(crate) success: Color,
    pub(crate) info: Color,
    pub(crate) highlight: Color,
    pub(crate) muted: Color,
    pub(crate) border: Color,
    pub(crate) panel_title: Color,
    pub(crate) app_bg: Color,
    pub(crate) overlay_bg: Color,
    pub(crate) scrim_bg: Color,
    pub(crate) key_bg: Color,
    pub(crate) key_bg_disabled: Color,
    pub(crate) key_bg_selected: Color,
    pub(crate) key_fg_selected: Color,
    pub(crate) row_highlight_bg: Color,
    pub(crate) toast_info_bg: Color,
    pub(crate) toast_info_icon: Color,
    pub(crate) toast_info_badge_bg: Color,
    pub(crate) toast_success_bg: Color,
    pub(crate) toast_success_icon: Color,
    pub(crate) toast_success_badge_bg: Color,
    pub(crate) toast_error_bg: Color,
    pub(crate) toast_error_icon: Color,
    pub(crate) toast_error_badge_bg: Color,
    pub(crate) shimmer_glow: Color,
    pub(crate) job_check_bg: Color,
    pub(crate) job_update_bg: Color,
    pub(crate) installed_text: Color,
    pub(crate) package_icon: Color,
}

impl UiTheme {
    pub(crate) fn storm() -> Self {
        Self {
            brand_hot: Color::Rgb(255, 95, 95),
            brand_warm: Color::Rgb(255, 158, 100),
            brand_gold: Color::Rgb(224, 175, 104),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            success: Color::Rgb(158, 206, 106),
            info: Color::Rgb(13, 185, 215),
            highlight: Color::Rgb(255, 158, 100),
            muted: Color::Rgb(86, 95, 137),
            border: Color::Rgb(54, 59, 84),
            panel_title: Color::Rgb(122, 162, 247),
            app_bg: Color::Rgb(36, 40, 59),
            overlay_bg: Color::Rgb(36, 40, 59),
            scrim_bg: Color::Rgb(26, 29, 44),
            key_bg: Color::Rgb(49, 54, 83),
            key_bg_disabled: Color::Rgb(41, 45, 66),
            key_bg_selected: Color::Rgb(40, 46, 67),
            key_fg_selected: Color::Rgb(172, 220, 255),
            row_highlight_bg: Color::Rgb(49, 54, 83),
            toast_info_bg: Color::Rgb(34, 55, 75),
            toast_info_icon: Color::Rgb(125, 207, 255),
            toast_info_badge_bg: Color::Rgb(25, 43, 60),
            toast_success_bg: Color::Rgb(42, 58, 46),
            toast_success_icon: Color::Rgb(196, 235, 144),
            toast_success_badge_bg: Color::Rgb(34, 49, 38),
            toast_error_bg: Color::Rgb(60, 42, 50),
            toast_error_icon: Color::Rgb(255, 182, 194),
            toast_error_badge_bg: Color::Rgb(52, 35, 43),
            shimmer_glow: Color::Rgb(255, 244, 214),
            job_check_bg: Color::Rgb(25, 41, 56),
            job_update_bg: Color::Rgb(48, 38, 28),
            installed_text: Color::Rgb(172, 182, 220),
            package_icon: Color::Rgb(212, 175, 55),
        }
    }

    pub(crate) fn day() -> Self {
        Self {
            brand_hot: Color::Rgb(245, 42, 101),
            brand_warm: Color::Rgb(177, 92, 0),
            brand_gold: Color::Rgb(140, 108, 62),
            warning: Color::Rgb(140, 108, 62),
            error: Color::Rgb(198, 67, 67),
            success: Color::Rgb(88, 117, 57),
            info: Color::Rgb(7, 135, 157),
            highlight: Color::Rgb(177, 92, 0),
            muted: Color::Rgb(132, 140, 181),
            border: Color::Rgb(166, 173, 207),
            panel_title: Color::Rgb(46, 125, 233),
            app_bg: Color::Rgb(225, 226, 231),
            overlay_bg: Color::Rgb(232, 233, 238),
            scrim_bg: Color::Rgb(212, 215, 224),
            key_bg: Color::Rgb(210, 218, 232),
            key_bg_disabled: Color::Rgb(221, 225, 234),
            key_bg_selected: Color::Rgb(203, 217, 224),
            key_fg_selected: Color::Rgb(7, 113, 151),
            row_highlight_bg: Color::Rgb(209, 214, 224),
            toast_info_bg: Color::Rgb(203, 217, 224),
            toast_info_icon: Color::Rgb(7, 135, 157),
            toast_info_badge_bg: Color::Rgb(191, 209, 218),
            toast_success_bg: Color::Rgb(214, 222, 204),
            toast_success_icon: Color::Rgb(88, 117, 57),
            toast_success_badge_bg: Color::Rgb(204, 214, 193),
            toast_error_bg: Color::Rgb(222, 210, 215),
            toast_error_icon: Color::Rgb(198, 67, 67),
            toast_error_badge_bg: Color::Rgb(214, 198, 204),
            shimmer_glow: Color::Rgb(248, 244, 232),
            job_check_bg: Color::Rgb(203, 217, 224),
            job_update_bg: Color::Rgb(226, 220, 209),
            installed_text: Color::Rgb(72, 89, 143),
            package_icon: Color::Rgb(177, 126, 0),
        }
    }

    pub(crate) fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::storm(),
            ThemeMode::Light => Self::day(),
        }
    }

    pub(crate) fn uses_light_surfaces(self) -> bool {
        matches!(self.app_bg, Color::Rgb(r, g, b) if r > 200 && g > 200 && b > 200)
    }

    pub(crate) fn modal_title_color(self) -> Color {
        if self.uses_light_surfaces() {
            self.panel_title
        } else {
            self.highlight
        }
    }
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::storm()
    }
}
