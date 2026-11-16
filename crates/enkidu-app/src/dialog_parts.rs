//! What every dialog of ours is made of (DESIGN.md, "Color" and "Space, size, shape"): a surface a step above the
//! window (`raised`, a 14 px corner, lit from above), a head with its title, a line under it and ×, a body, and a foot
//! band with its buttons. The library's own dialog would stand on the islands' color with their 12 px corner and no lit
//! edge.

use enkidu_ui::pieces::Observed;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Pixels, Role, SharedString, div, px, rgb};

/// The corner of what fills a dialog to its edge (a foot band, a side column): the dialog's own, less its 1 px edge.
/// Nothing clips to a rounded corner here, so such a fill rounds itself.
pub const INNER_RADIUS: Pixels = px(13.);

/// The surface: `raised`, the dialogs' corner, no padding and no × of the library's (the view draws its own head).
pub fn surface(dialog: Dialog) -> Dialog {
    dialog
        .p(px(0.))
        .bg(theme::RAISED)
        .rounded(theme::RADIUS_DIALOG)
        .close_button(false)
        // it fades in over its last 8 px, 220 ms (the theme's motion). The palette and Search, opened from the
        // keyboard many times a day, set `DialogEntrance::None` after this
        .entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide)
}

/// The × of a dialog's head. Who uses it says what a press does.
pub fn close(id: &'static str) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(Role::Button)
        .aria_label("Close")
        .flex_none()
        .mr(px(-6.))
        .mt(px(-2.))
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER))
        .child(tinted("icons/x.svg", theme::INK_3, 14.))
}

/// The head: the title (15/20 semibold), a line under it on what the dialog is for, and what closes it at the right.
pub fn head(title: impl Into<SharedString>, says: Option<SharedString>, close: Option<Observed>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(16.))
        .px(px(20.))
        .pt(px(18.))
        .pb(px(4.))
        .child(
            div()
                .min_w_0()
                .child(
                    div()
                        .text_size(theme::DIALOG_TITLE)
                        .line_height(theme::DIALOG_TITLE_LINE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .truncate()
                        .child(title.into()),
                )
                .children(says.map(|says| {
                    div()
                        .mt(px(4.))
                        .text_size(theme::TEXT_SMALL)
                        .line_height(theme::TEXT_SMALL_LINE)
                        .text_color(theme::INK_2)
                        .child(says)
                })),
        )
        .children(close)
}

/// The lit top edge of a dialog, its last child (the root is `relative`): the islands' edge, kept clear of the
/// dialog's wider corner.
pub fn lit_edge() -> Div {
    enkidu_ui::pieces::lit_edge().left(theme::RADIUS_DIALOG).right(theme::RADIUS_DIALOG)
}

/// The foot band: a hairline above it, a shade darker, its buttons at the right (put what is said first, with
/// `flex_1`, to push them there).
pub fn foot() -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .border_t_1()
        .border_color(theme::RULE)
        .bg(alpha(rgb(0x000000), 0.2))
        .rounded_b(INNER_RADIUS)
        .px(px(20.))
        .py(px(12.))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AbsoluteLength, Fill, TestAppContext};

    /// DESIGN.md, "Color": `raised` is for "menus, dialogs, the palette, toasts, remark cards"; "Space, size, shape":
    /// radius 14 for "dialogs, the palette, Search".
    #[gpui_kit::test]
    fn a_dialog_stands_on_the_raised_surface_with_the_dialogs_corner(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let mut dialog = surface(Dialog::new(cx));
            let style = dialog.style();
            assert_eq!(style.background, Some(Fill::from(theme::RAISED)));
            assert_eq!(theme::RAISED, gpui_kit::rgb(0x191b21));
            let corner = Some(AbsoluteLength::Pixels(px(14.)));
            let radii = &style.corner_radii;
            assert_eq!([radii.top_left, radii.top_right, radii.bottom_left, radii.bottom_right], [corner; 4]);
            assert_eq!(INNER_RADIUS, theme::RADIUS_DIALOG - px(1.));
        });
    }

    /// Every dialog the app opens is built on [`surface`]: one that forgets it would stand on the islands' color with
    /// their corner, as New workspace, Remove workspace, Settings, Search and the palette did until item 7.1a.
    #[test]
    fn every_dialog_the_app_opens_is_built_on_the_shared_surface() {
        fn rust_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rust_files(&path, found);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(path);
                }
            }
        }
        let mut files = Vec::new();
        rust_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let opens = [".open", "_dialog("].concat();
        let mut opened = 0;
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (at, _) in text.match_indices(&opens) {
                opened += 1;
                let builder: String = text[at..].chars().take(900).collect();
                assert!(
                    builder.contains("dialog_parts::surface(dialog)"),
                    "{}: a dialog off the surface",
                    file.display()
                );
            }
        }
        assert!(opened >= 8, "the eight dialogs were looked at, found {opened}");
    }
}
