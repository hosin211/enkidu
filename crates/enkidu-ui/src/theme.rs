//! gpui-kit's theme, set from our tokens so its controls (buttons, inputs, lists, tables, menus, dialogs, tabs,
//! scrollbars) match DESIGN.md (item 1.10). Dark only: there is no light theme.
//!
//! Every one of gpui-kit's 139 colors is set here; a test checks that each one is a color of our palette.

use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeColor, ThemeMode};

use crate::tokens::*;

/// Our colors for every part of gpui-kit's theme.
pub fn colors() -> ThemeColor {
    let h = hsla;
    let none = h(white(0.0));
    let mut c = ThemeColor::default();
    // the layers and their text
    c.background = h(WELL);
    c.foreground = h(INK);
    c.border = h(LINE_STRONG);
    c.window_border = h(LINE);
    c.popover = h(SHEET);
    c.popover_foreground = h(INK);
    c.overlay = h(SCRIM);
    c.muted = h(HOVER);
    c.muted_foreground = h(INK_3);
    // gpui-kit's "accent" is the hover fill of menu and list rows; our signal blue is its ring, caret and links
    c.accent = h(HOVER);
    c.accent_foreground = h(INK);
    c.ring = h(FOCUS);
    c.caret = h(ACCENT);
    c.selection = h(ACCENT_SOFT);
    c.input = h(LINE_STRONG);
    c.link = h(ACCENT);
    c.link_hover = h(ACCENT);
    c.link_active = h(ACCENT_STRONG);
    c.drag_border = h(ACCENT);
    c.drop_target = h(ACCENT_SOFT);
    c.progress_bar = h(ACCENT);
    c.skeleton = h(HOVER);
    // buttons: primary is white with black words; secondary has a faint fill; info is ours
    c.primary = h(INK);
    c.primary_hover = h(white(1.0));
    c.primary_active = h(INK_2);
    c.primary_foreground = h(ON_INK);
    c.secondary = h(HOVER);
    c.secondary_hover = h(SOFT);
    c.secondary_active = h(THUMB);
    c.secondary_foreground = h(INK);
    c.button = h(HOVER);
    c.button_hover = h(SOFT);
    c.button_active = h(THUMB);
    c.button_foreground = h(INK);
    c.button_primary = h(INK);
    c.button_primary_hover = h(white(1.0));
    c.button_primary_active = h(INK_2);
    c.button_primary_foreground = h(ON_INK);
    c.button_secondary = h(HOVER);
    c.button_secondary_hover = h(SOFT);
    c.button_secondary_active = h(THUMB);
    c.button_secondary_foreground = h(INK);
    c.info = h(ACCENT);
    c.info_hover = h(ACCENT);
    c.info_active = h(ACCENT_STRONG);
    c.info_foreground = h(ON_INK);
    c.button_info = h(ACCENT);
    c.button_info_hover = h(ACCENT);
    c.button_info_active = h(ACCENT_STRONG);
    c.button_info_foreground = h(ON_INK);
    // the lamps: done, working, needs you
    for (base, hover, active, fg, color) in [
        (&mut c.success, &mut c.success_hover, &mut c.success_active, &mut c.success_foreground, LAMP_GREEN),
        (&mut c.warning, &mut c.warning_hover, &mut c.warning_active, &mut c.warning_foreground, LAMP_AMBER),
        (&mut c.danger, &mut c.danger_hover, &mut c.danger_active, &mut c.danger_foreground, LAMP_RED),
    ] {
        (*base, *hover, *active, *fg) = (h(color), h(color), h(color), h(ON_INK));
    }
    for (base, hover, active, fg, color) in [
        (
            &mut c.button_success,
            &mut c.button_success_hover,
            &mut c.button_success_active,
            &mut c.button_success_foreground,
            LAMP_GREEN,
        ),
        (
            &mut c.button_warning,
            &mut c.button_warning_hover,
            &mut c.button_warning_active,
            &mut c.button_warning_foreground,
            LAMP_AMBER,
        ),
        (
            &mut c.button_danger,
            &mut c.button_danger_hover,
            &mut c.button_danger_active,
            &mut c.button_danger_foreground,
            LAMP_RED,
        ),
    ] {
        (*base, *hover, *active, *fg) = (h(color), h(color), h(color), h(ON_INK));
    }
    // lists and tables: the selected row is a fill with a signal-blue bar
    c.list = none;
    c.list_even = none;
    c.list_head = h(PANE);
    c.list_hover = h(HOVER);
    c.list_active = h(SELECTED);
    c.list_active_border = h(ACCENT);
    c.table = none;
    c.table_even = none;
    c.table_head = h(PANE);
    c.table_head_foreground = h(INK_2);
    c.table_foot = h(PANE);
    c.table_foot_foreground = h(INK_2);
    c.table_hover = h(HOVER);
    c.table_active = h(SELECTED);
    c.table_active_border = h(ACCENT);
    c.table_row_border = h(LINE);
    c.description_list_label = h(PANE);
    c.description_list_label_foreground = h(INK_2);
    c.group_box = h(PANE);
    c.group_box_foreground = h(INK);
    c.accordion = h(WELL);
    // tabs, bars and the side rail: the frame around the islands
    c.tab_bar = h(PANE);
    c.tab_bar_segmented = h(PANE);
    c.tab = none;
    c.tab_active = h(WELL);
    c.tab_foreground = h(INK_2);
    c.tab_active_foreground = h(INK);
    c.title_bar = h(PANE);
    c.title_bar_border = h(LINE);
    c.status_bar = h(PANE);
    c.status_bar_border = h(LINE);
    c.sidebar = h(PANE);
    c.sidebar_border = h(LINE);
    c.sidebar_foreground = h(INK_2);
    c.sidebar_accent = h(HOVER);
    c.sidebar_accent_foreground = h(INK);
    c.sidebar_primary = h(ACCENT);
    c.sidebar_primary_foreground = h(ON_INK);
    // controls
    c.scrollbar = none;
    c.scrollbar_thumb = h(THUMB);
    c.scrollbar_thumb_hover = h(LINE_STRONG);
    c.slider_bar = h(SOFT);
    c.slider_thumb = h(INK);
    c.switch = h(SOFT);
    c.switch_thumb = h(INK);
    // named hues and charts, from the lamps and the accent
    c.red = h(LAMP_RED);
    c.red_light = h(DEL_BG);
    c.green = h(LAMP_GREEN);
    c.green_light = h(ADD_BG);
    c.blue = h(ACCENT);
    c.blue_light = h(ACCENT_SOFT);
    c.yellow = h(LAMP_AMBER);
    c.yellow_light = h(alpha(LAMP_AMBER, 0.14));
    c.magenta = h(DEL_INK);
    c.magenta_light = h(DEL_BG);
    c.cyan = h(ADD_INK);
    c.cyan_light = h(ADD_BG);
    c.chart_bullish = h(LAMP_GREEN);
    c.chart_bearish = h(LAMP_RED);
    c.chart_grid = h(LINE);
    c.chart_1 = h(ACCENT);
    c.chart_2 = h(LAMP_GREEN);
    c.chart_3 = h(LAMP_AMBER);
    c.chart_4 = h(LAMP_RED);
    c.chart_5 = h(CLAUDE);
    c
}

/// Sets gpui-kit's theme to ours: dark, our colors, our faces and sizes, our corners. Call once after
/// `gpui_kit::init`.
pub fn apply(cx: &mut App) {
    // switching the mode loads gpui-kit's own dark theme, so ours goes on in a second step (Theme::update's docs)
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.colors = colors();
        theme.font_family = UI_FONT.into();
        theme.font_size = TEXT;
        theme.mono_font_family = MONO_FONT.into();
        theme.mono_font_size = MONO_TEXT;
        // what the library rounds with this is what floats (its menus, the list of a select); our own controls and
        // fields round themselves
        theme.radius = RADIUS_DIALOG;
        theme.radius_lg = RADIUS_DIALOG;
        theme.shadow = true;
        // what the library moves (a dialog coming in) moves by the design's numbers: 220 ms, our strong ease-out,
        // 8 px of travel (DESIGN.md, "Motion": "Dialogs: rise 8 px and fade, 220 ms")
        theme.motion.duration_fast = DUR_PRESS;
        theme.motion.duration_normal = DUR_BASE;
        theme.motion.duration_slow = DUR_SLOW;
        let [x1, y1, x2, y2] = EASE_OUT;
        if let Ok(curve) = gpui_kit::base::Easing::cubic_bezier(x1, y1, x2, y2) {
            theme.motion.easing_enter = curve;
        }
        theme.motion.distance_short = gpui_kit::rems(DIALOG_TRAVEL / f32::from(TEXT));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Hsla, TestAppContext};

    /// Every color our palette can give gpui-kit.
    fn palette() -> Vec<Hsla> {
        [
            PANE,
            WELL,
            SHEET,
            TIP,
            INK,
            INK_2,
            INK_3,
            ON_INK,
            LINE,
            LINE_STRONG,
            HOVER,
            SOFT,
            SELECTED,
            THUMB,
            ACCENT,
            ACCENT_STRONG,
            ACCENT_SOFT,
            LAMP_AMBER,
            LAMP_RED,
            LAMP_GREEN,
            ADD_BG,
            ADD_INK,
            DEL_BG,
            DEL_INK,
            SCRIM,
            white(0.0),
            white(1.0),
            alpha(LAMP_AMBER, 0.14),
            CLAUDE,
        ]
        .into_iter()
        .map(hsla)
        .collect()
    }

    #[test]
    fn every_one_of_gpui_kits_colors_is_ours() {
        let ours = palette();
        let set = serde_json::to_value(colors()).unwrap();
        let fields = set.as_object().unwrap();
        assert_eq!(fields.len(), 139, "gpui-kit's theme has changed its colors");
        let near = |a: &Hsla, b: &Hsla| {
            (a.a - b.a).abs() < 0.002
                && (a.a < 0.002
                    || ((a.h - b.h).abs() < 0.002 && (a.s - b.s).abs() < 0.002 && (a.l - b.l).abs() < 0.002))
        };
        let mut clear = Vec::new();
        for (name, value) in fields {
            let color: Hsla = serde_json::from_value(value.clone()).unwrap();
            assert!(ours.iter().any(|o| near(o, &color)), "{name} is not one of our colors: {color:?}");
            if color.a < 0.002 {
                clear.push(name.as_str());
            }
        }
        // a color left at gpui-kit's default is clear too: only these are clear on purpose
        clear.sort();
        assert_eq!(clear, ["list", "list_even", "scrollbar", "tab", "table", "table_even"]);
    }

    #[gpui_kit::test]
    fn the_app_gets_our_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(cx);
            let theme = Theme::global(cx);
            assert!(theme.is_dark());
            assert_eq!(theme.colors, colors());
            assert_eq!(theme.background, hsla(WELL));
            assert_eq!(theme.popover, hsla(SHEET));
            assert_eq!(theme.ring, hsla(ACCENT));
            assert_eq!(theme.primary, hsla(INK), "primary buttons are white");
            assert_eq!(theme.font_family.as_ref(), ".SystemUIFont");
            assert_eq!(theme.mono_font_family.as_ref(), "JetBrains Mono");
            assert_eq!((theme.radius, theme.radius_lg), (RADIUS_DIALOG, RADIUS_DIALOG));
        });
    }
}
