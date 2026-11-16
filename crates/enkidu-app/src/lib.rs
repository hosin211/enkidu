//! Enkidu: the GPUI app.

pub mod bridge;
pub mod changes_island;
pub mod definitions;
pub mod demo;
mod dialog_parts;
pub mod empty;
pub mod files_pane;
pub mod keymap;
pub mod live;
pub mod measure;
pub mod new_workspace;
pub mod opened_file;
pub mod palette;
pub mod projects_row;
pub mod remark_views;
pub mod remarks;
pub mod remove_workspace;
pub mod right_edge;
pub mod search;
pub mod settings;
mod settings_work;
pub mod setup;
pub mod shell;
pub mod status_line;
pub mod stores;
pub mod tabs_row;
pub mod terminal_area;
pub mod terminal_view;
pub mod terminals;
pub mod toast;
pub mod window_state;
pub mod work;

use gpui_kit::{
    App, Bounds, KeyBinding, Menu, MenuItem, NoAction, OsAction, SystemMenuType, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, point, px, size,
};

/// The app's identity: its data folder is named after it, and macOS knows the app by it.
pub const APP_ID: &str = "io.github.hosin211.enkidu";
pub const APP_NAME: &str = "Enkidu";

actions!(
    enkidu,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen,
        BringAllToFront,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        SelectAll,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectTab9,
        NextWorkspace,
        PreviousWorkspace,
        NewWorkspace,
        TogglePalette,
        TypeRemarks,
        CloseFile,
        GoBack,
        GoForward,
        ToggleSideLayout,
        ToggleTerminalOnly,
        ToggleFullReview,
        ToggleChangedFiles,
        ToggleFiles,
        OpenSearch,
        SearchFiles,
        SearchSymbols,
        SearchText,
        ToggleMatchCase,
        ToggleWholeWords,
        ToggleRegex,
        OpenSettings,
        ToggleWork,
        TextBigger,
        TextSmaller,
        TextUsualSize
    ]
);

/// The Mac's usual keys for the menus' items. Copy and Paste are the terminal's own when it has the keyboard;
/// Undo, Redo, Cut and Select All have nothing to act on yet, so their items stay greyed out (item 1.14).
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("shift-cmd-z", Redo, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        // the tabs (DESIGN.md, "Layout"): ⌘1 to ⌘9, ⌃⇥ and ⌃⇧⇥, ⇧⌘] and ⇧⌘[ (which the Mac reports as ⌘} and ⌘{)
        KeyBinding::new("cmd-1", SelectTab1, None),
        KeyBinding::new("cmd-2", SelectTab2, None),
        KeyBinding::new("cmd-3", SelectTab3, None),
        KeyBinding::new("cmd-4", SelectTab4, None),
        KeyBinding::new("cmd-5", SelectTab5, None),
        KeyBinding::new("cmd-6", SelectTab6, None),
        KeyBinding::new("cmd-7", SelectTab7, None),
        KeyBinding::new("cmd-8", SelectTab8, None),
        KeyBinding::new("cmd-9", SelectTab9, None),
        KeyBinding::new("ctrl-tab", NextWorkspace, None),
        KeyBinding::new("ctrl-shift-tab", PreviousWorkspace, None),
        KeyBinding::new("cmd-}", NextWorkspace, None),
        KeyBinding::new("cmd-{", PreviousWorkspace, None),
        // the New workspace dialog for the project on screen
        KeyBinding::new("cmd-n", NewWorkspace, None),
        // the palette: every workspace of every project, and the actions
        KeyBinding::new("cmd-k", TogglePalette, None),
        // the layouts (DESIGN.md, "Layout"): the changes beside the terminal or above it, the terminal alone, the
        // changes alone
        KeyBinding::new("alt-cmd-l", ToggleSideLayout, None),
        KeyBinding::new("alt-cmd-b", ToggleTerminalOnly, None),
        KeyBinding::new("cmd-shift-e", ToggleFullReview, None),
        // the panels at the right edge (DESIGN.md, "The right edge is a tool bar"): Changed files and Files
        // the text of what has the keyboard (item 8.4): ⌘+ (and ⌘=, the same key without Shift), ⌘− and ⌘0
        KeyBinding::new("cmd-=", TextBigger, None),
        KeyBinding::new("cmd-+", TextBigger, None),
        KeyBinding::new("cmd--", TextSmaller, None),
        KeyBinding::new("cmd-0", TextUsualSize, None),
        KeyBinding::new("alt-cmd-1", ToggleChangedFiles, None),
        KeyBinding::new("alt-cmd-2", ToggleFiles, None),
        // with the keyboard in the changes: the remarks go into Claude's prompt (a text field's own ⌘↩ comes first, and
        // in a terminal the key is the terminal's)
        KeyBinding::new("cmd-enter", TypeRemarks, Some(changes_island::KEYS)),
        // and Escape there leaves an opened file
        KeyBinding::new("escape", CloseFile, Some(changes_island::KEYS)),
        // after a ⌘-click went to a definition (item 10.11): back to where it was made, and forward again, as an
        // IDE has them
        KeyBinding::new("cmd-[", GoBack, Some(changes_island::KEYS)),
        KeyBinding::new("cmd-]", GoForward, Some(changes_island::KEYS)),
        // Tab and Shift-Tab in a terminal are the program's: the shell completes a name with one, Claude changes its
        // mode with the other. gpui-kit's root binds both to "the next control" for everything inside it, and a
        // binding is asked before a key listener, so they never reached the terminal. Here
        // the terminal says they are bound to nothing inside it, and the key goes on to its listener.
        KeyBinding::new("tab", NoAction, Some(terminal_view::KEYS)),
        KeyBinding::new("shift-tab", NoAction, Some(terminal_view::KEYS)),
        // Settings, as every Mac app has them
        KeyBinding::new("cmd-,", OpenSettings, None),
        // the Work page (DESIGN.md, "The Work page"): ⌘I opens it and goes back; with the keyboard on the page, Esc
        // goes back too
        KeyBinding::new("cmd-i", ToggleWork, None),
        KeyBinding::new("escape", work::page::Back, Some(work::page::KEYS)),
        // on the board: the arrows (or h j k l) move from card to card, ↩ opens one, X picks it, / filters. In the
        // filter's field they are letters, and ↓ leaves it for the board
        KeyBinding::new("up", work::page::MoveUp, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("k", work::page::MoveUp, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("down", work::page::MoveDown, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("j", work::page::MoveDown, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("left", work::page::MoveLeft, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("h", work::page::MoveLeft, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("right", work::page::MoveRight, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("l", work::page::MoveRight, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("enter", work::page::OpenCard, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("space", work::page::OpenCard, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("x", work::page::PickCard, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("/", work::page::FocusFilter, Some(work::page::BOARD_KEYS)),
        KeyBinding::new("down", work::page::FilterDown, Some(work::page::FILTER_KEYS)),
        // in front of a card: through what needs you, and what to do with the item the keyboard is on (in a text
        // field these are letters)
        KeyBinding::new("j", work::card::NextItem, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("down", work::card::NextItem, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("k", work::card::PrevItem, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("up", work::card::PrevItem, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("x", work::card::ToggleItem, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("n", work::card::ItemNote, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("e", work::card::ItemDone, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("z", work::card::ItemSnooze, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("enter", work::card::OpenItem, Some(work::card::PLAIN_KEYS)),
        // s (or c) writes to Claude; ⌘↩ types the card's message into its prompt, from the card itself (in a field
        // the field says so)
        KeyBinding::new("s", work::card::TellClaude, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("c", work::card::TellClaude, Some(work::card::PLAIN_KEYS)),
        KeyBinding::new("cmd-enter", work::card::SendTask, Some(work::card::PLAIN_KEYS)),
        // a report's screenshot shown as large as the window: → and ← go through the report's others (around the
        // ends), Esc closes it, not the card under it
        // the Workflows page, in the board's place: through its tasks, and back to the board
        KeyBinding::new("down", work::flows_page::NextTask, Some(work::flows_page::PLAIN_KEYS)),
        KeyBinding::new("j", work::flows_page::NextTask, Some(work::flows_page::PLAIN_KEYS)),
        KeyBinding::new("up", work::flows_page::PrevTask, Some(work::flows_page::PLAIN_KEYS)),
        KeyBinding::new("k", work::flows_page::PrevTask, Some(work::flows_page::PLAIN_KEYS)),
        KeyBinding::new("escape", work::flows_page::LeaveFlows, Some(work::flows_page::KEYS)),
        KeyBinding::new("right", work::shots::NextShot, Some(work::shots::KEYS)),
        KeyBinding::new("left", work::shots::PrevShot, Some(work::shots::KEYS)),
        KeyBinding::new("escape", work::shots::CloseShot, Some(work::shots::KEYS)),
        // Search (DESIGN.md, "Search"): Files by ⇧⌘O or ⌘P, Symbols by ⌘O (or ⌥⌘O), Text by ⇧⌘F. ⇧⇧
        // has no binding: the window hears Shift itself, to count two clean taps
        KeyBinding::new("cmd-shift-o", SearchFiles, None),
        KeyBinding::new("cmd-p", SearchFiles, None),
        KeyBinding::new("cmd-o", SearchSymbols, None),
        KeyBinding::new("alt-cmd-o", SearchSymbols, None),
        KeyBinding::new("cmd-shift-f", SearchText, None),
        // Find in files' three switches, with the same keys as VS Code, while Search is open
        KeyBinding::new("alt-cmd-c", ToggleMatchCase, Some(search::KEYS)),
        KeyBinding::new("alt-cmd-w", ToggleWholeWords, Some(search::KEYS)),
        KeyBinding::new("alt-cmd-r", ToggleRegex, Some(search::KEYS)),
    ]
}

/// The Mac's menus: the app's own, Edit, View and Window, as a Mac app has them.
pub fn menus() -> Vec<Menu> {
    let menu = |name: &str, items: Vec<MenuItem>| Menu { name: name.to_string().into(), items, disabled: false };
    vec![
        menu(
            APP_NAME,
            vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Enkidu", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Enkidu", Quit),
            ],
        ),
        menu(
            "Edit",
            vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ],
        ),
        menu("View", vec![MenuItem::action("Enter Full Screen", ToggleFullScreen)]),
        menu(
            "Window",
            vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Bring All to Front", BringAllToFront),
            ],
        ),
    ]
}

/// Runs `act` on the window in front, if there is one.
fn front_window(cx: &mut App, act: impl FnOnce(&mut Window)) {
    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_, window, _| act(window));
    }
}

/// Sets up what every window needs: gpui-kit, the fonts, the keys and the menus.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    // the keys of the diff view (our own copy of gpui-kit's, `enkidu-diff`)
    enkidu_diff::init(cx);
    // gpui-kit's controls in our look: dark, our colors, faces and corners (DESIGN.md; item 1.10)
    enkidu_ui::theme::apply(cx);
    if let Err(err) = enkidu_ui::assets::load_fonts(cx) {
        eprintln!("Enkidu: the terminal font could not be loaded: {err}");
    }
    cx.bind_keys(key_bindings());
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(true));
    cx.on_action(|_: &Minimize, cx| front_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| front_window(cx, |window| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| front_window(cx, |window| window.toggle_fullscreen()));
    cx.set_menus(menus());
}

/// The main window: where it was left (else 1360 by 860, centred), the title bar see-through, the traffic lights in the
/// projects row, never smaller than 900 by 560. `focus` is false for a window opened in the background for a
/// measurement.
pub fn window_options(cx: &App, focus: bool) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(window_state::FIRST.0), px(window_state::FIRST.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(enkidu_ui::tokens::TRAFFIC_LIGHTS.0),
                px(enkidu_ui::tokens::TRAFFIC_LIGHTS.1),
            )),
        }),
        focus,
        show: true,
        window_min_size: Some(size(px(window_state::MIN.0), px(window_state::MIN.1))),
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}
