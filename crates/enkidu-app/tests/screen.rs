//! Screen tests: the real views in a headless window, driven with real clicks, keys, the wheel and drags. No window
//! appears on screen. The terminal in these windows runs nothing; what it would send to a program is kept, so the
//! tests read the exact bytes.

use std::sync::Arc;

use enkidu_app::shell::Shell;
use enkidu_term::Terminal;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ClipboardItem, Entity, ScrollDelta, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Shell>) {
    cx.update(|cx| {
        enkidu_app::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1360.), px(860.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::demo(window, cx)),
        )
        .expect("a test window")
    })
}

fn terminal_of(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Arc<Terminal> {
    shell.read_with(cx, |shell, cx| shell.terminal().read(cx).terminal().clone())
}

#[gpui_kit::test]
fn a_click_on_a_tab_selects_it(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(("tab", 0usize)).selected(), Some(true));
        let second = window.find(("tab", 1usize));
        assert_eq!(second.label(), Some("sh-21309"));
        assert_eq!(second.selected(), Some(false));
        window.click(("tab", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(window.find(("tab", 1usize)).selected(), Some(true));
        assert_eq!(window.find(("tab", 0usize)).selected(), Some(false));
    })
    .unwrap();
    assert_eq!(shell.read_with(cx, |shell, cx| shell.selected(cx)), 1);
}

#[gpui_kit::test]
fn the_frame_has_the_designs_sizes(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("projects-row").bounds().size.height, px(38.));
        assert_eq!(window.find("tabs-row").bounds().size.height, px(40.));
        assert_eq!(window.find("status-line").bounds().size.height, px(26.));
        assert_eq!(window.find(("tab", 0usize)).bounds().size.height, px(30.));
        let island = window.find("island-claude").bounds();
        // 8 px from the window's sides, between the tabs row and the status line
        assert_eq!(island.origin.x, px(8.));
        assert_eq!(island.size.width, px(1360. - 16.));
        assert_eq!(island.origin.y, px(78.));
        assert_eq!(island.size.height, px(860. - 78. - 26.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn keys_reach_the_program_as_claude_code_reads_them(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal").focused(), Some(true));
        for key in
            ["h", "i", "enter", "shift-enter", "escape", "ctrl-c", "up", "down", "left", "right", "alt-b", "ctrl-v"]
        {
            window.press(key, cx);
        }
    })
    .unwrap();
    assert_eq!(terminal.sent(), b"hi\r\x1b\r\x1b\x03\x1b[A\x1b[B\x1b[D\x1b[C\x1bb\x16");
}

#[gpui_kit::test]
fn with_the_kitty_protocol_on_keys_are_sent_its_way(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // The program turns on the kitty keyboard protocol, as Claude Code can.
    terminal.feed(b"\x1b[>1u");
    let before = terminal.sent().len();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        for key in ["shift-enter", "escape", "ctrl-c", "a", "enter"] {
            window.press(key, cx);
        }
    })
    .unwrap();
    assert_eq!(&terminal.sent()[before..], b"\x1b[13;2u\x1b[27u\x1b[99;5ua\r");
}

#[gpui_kit::test]
fn a_paste_goes_in_as_one_block(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("first line\nsecond line".into())));
    // Claude Code and shells turn on bracketed paste.
    terminal.feed(b"\x1b[?2004h");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-v", cx);
    })
    .unwrap();
    assert_eq!(terminal.sent(), b"\x1b[200~first line\rsecond line\x1b[201~");
}

#[gpui_kit::test]
fn the_trackpad_scrolls_back_through_the_history(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    for i in 0..300 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    let line_height = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn").line_height
        })
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        // Two small trackpad moves of 2.5 lines each make 5 whole lines back.
        for _ in 0..2 {
            window.scroll("terminal", ScrollDelta::Pixels(point(px(0.), line_height * 2.5)), cx);
        }
        window.render_frame(cx);
    })
    .unwrap();
    let screen = terminal.screen();
    assert_eq!(screen.display_offset, 5);
    assert!(screen.history > 200);
    cx.update_window(window, |_, window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., -5.)), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(terminal.screen().display_offset, 0);
}

#[gpui_kit::test]
fn a_drag_selects_text_and_cmd_c_copies_it(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[2J\x1b[Hhello world\r\n");
    let layout = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn")
        })
        .unwrap();
    let at = |col: f32, row: f32| {
        point(layout.origin.x + layout.cell_width * col, layout.origin.y + layout.line_height * row)
    };
    cx.update_window(window, |_, window, cx| {
        window.drag(at(6.2, 0.5), at(10.8, 0.5), cx);
        window.render_frame(cx);
        window.press("cmd-c", cx);
    })
    .unwrap();
    assert_eq!(terminal.selection_text().as_deref(), Some("world"));
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("world"));
    // Cmd+C copies; it sends nothing to the program.
    assert!(terminal.sent().is_empty());
}

#[gpui_kit::test]
fn the_terminal_follows_the_window_size(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let wide = terminal.size();
    cx.simulate_window_resize(window, size(px(1000.), px(640.)));
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let narrow = terminal.size();
    assert!(narrow.cols < wide.cols && narrow.rows < wide.rows, "{wide:?} then {narrow:?}");
    let screen = terminal.screen();
    assert_eq!((screen.cols, screen.rows), (usize::from(narrow.cols), usize::from(narrow.rows)));
}

#[gpui_kit::test]
fn the_tab_pill_glides_to_the_chosen_tab(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    // the tabs report their places as they are drawn; the pill goes there from the next frame on
    let frame = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let tab = |ix: usize| window.find(("tab", ix)).bounds();
            (window.find("workspace-tabs-pill").bounds(), tab(0), tab(1))
        })
        .unwrap()
    };
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let (pill, first, second) = frame(cx);
    assert_eq!(pill, first, "on the tab on screen");
    cx.update_window(window, |_, window, cx| window.click(("tab", 1usize), cx)).unwrap();
    frame(cx);
    // halfway through its 220 ms it is between the two tabs, past the middle (a strong ease-out)
    cx.executor().advance_clock(std::time::Duration::from_millis(110));
    let (pill, ..) = frame(cx);
    let middle = (first.origin.x + second.origin.x) / 2.;
    assert!(pill.origin.x > middle && pill.origin.x < second.origin.x, "{pill:?} between {first:?} and {second:?}");
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    let (pill, _, second) = frame(cx);
    assert_eq!(pill, second, "it settles on the chosen tab");
}

#[gpui_kit::test]
fn the_menus_are_the_macs_with_their_usual_keys(cx: &mut TestAppContext) {
    let names: Vec<String> = enkidu_app::menus().iter().map(|m| m.name.to_string()).collect();
    assert_eq!(names, ["Enkidu", "Edit", "View", "Window"]);
    cx.update(|cx| {
        enkidu_app::init(cx);
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let keys = |action: &dyn gpui_kit::Action| -> Vec<String> {
            keymap
                .bindings_for_action(action)
                .flat_map(|b| b.keystrokes().iter().map(|k| k.unparse(gpui_kit::OperatingSystem::Mac)))
                .collect()
        };
        assert_eq!(keys(&enkidu_app::Quit), ["cmd-q"]);
        assert_eq!(keys(&enkidu_app::Hide), ["cmd-h"]);
        assert_eq!(keys(&enkidu_app::HideOthers), ["alt-cmd-h"]);
        assert_eq!(keys(&enkidu_app::Minimize), ["cmd-m"]);
        assert_eq!(keys(&enkidu_app::ToggleFullScreen), ["ctrl-cmd-f"]);
        assert_eq!(keys(&enkidu_app::Copy), ["cmd-c"]);
        assert_eq!(keys(&enkidu_app::Paste), ["cmd-v"]);
        assert_eq!(keys(&enkidu_app::Undo), ["cmd-z"]);
        assert_eq!(keys(&enkidu_app::Redo), ["cmd-shift-z"], "GPUI writes it cmd first");
    });
}

#[gpui_kit::test]
fn edit_copy_and_paste_act_on_the_terminal(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[2J\x1b[Hhello world\r\n");
    let layout = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn")
        })
        .unwrap();
    let at = |col: f32, row: f32| {
        point(layout.origin.x + layout.cell_width * col, layout.origin.y + layout.line_height * row)
    };
    // as the Edit menu does it: the actions themselves, no keys
    cx.update_window(window, |_, window, cx| {
        window.drag(at(0.2, 0.5), at(4.8, 0.5), cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(enkidu_app::Copy), cx);
    })
    .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("hello"));
    cx.update_window(window, |_, window, cx| window.dispatch_action(Box::new(enkidu_app::Paste), cx)).unwrap();
    assert_eq!(terminal.sent(), b"hello".to_vec(), "Paste sends the clipboard to the program");
}

/// A view that only keeps its window's place, as the app's window does (item 1.14).
struct Remembers;

impl gpui_kit::Render for Remembers {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

#[gpui_kit::test]
fn the_window_keeps_its_size_a_moment_after_it_changes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("window.json");
    let kept = path.clone();
    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(100.), px(80.)),
                        size: size(px(1360.), px(860.)),
                    })),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        enkidu_app::window_state::remember(kept, window, cx);
                        Remembers
                    })
                },
            )
        })
        .unwrap();
    cx.executor().allow_parking();
    cx.simulate_window_resize(window.into(), size(px(1500.), px(940.)));
    // not at once: only once it has stayed that size for a moment
    cx.run_until_parked();
    assert!(!path.exists(), "written while still changing");
    cx.executor().advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    let saved = enkidu_app::window_state::load(&path).expect("kept");
    assert_eq!((saved.width, saved.height), (1500., 940.));
    assert!(!saved.maximized && !saved.fullscreen);
}

#[gpui_kit::test]
fn the_program_hears_when_the_terminal_gains_and_loses_the_keyboard(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // a program that asked to hear it (`CSI ? 1004 h`)
    terminal.feed(b"\x1b[?1004h");
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.update_window(window, |_, window, _| window.activate_window()).unwrap();
    cx.run_until_parked();
    assert_eq!(terminal.sent(), b"\x1b[I", "the window came to the front with the terminal focused");
    // the palette opens and takes the keyboard (with a terminal on screen the keyboard is never nowhere: the shell
    // gives it back to the terminal, so something else has to hold it)
    cx.update_window(window, |_, window, cx| {
        window.press("cmd-k", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(terminal.sent(), b"\x1b[I\x1b[O", "the palette has the keyboard");
    // a click on the terminal, outside the palette, closes it and gives the keyboard back
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("terminal", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(terminal.sent(), b"\x1b[I\x1b[O\x1b[I", "a click gave it back");
}

#[gpui_kit::test]
fn a_program_that_takes_the_mouse_hears_clicks_and_the_wheel_instead_of_a_selection(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // presses and releases, written the SGR way, as full-screen programs ask
    terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    let (x, y) = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let middle = window.find("terminal").bounds().center();
            let (row, col, _) = shell.read(cx).terminal().read(cx).layout().expect("drawn").cell_at(middle);
            window.click("terminal", cx);
            window.right_click("terminal", cx);
            window.scroll("terminal", ScrollDelta::Lines(point(0., 2.)), cx);
            (col + 1, row + 1)
        })
        .unwrap();
    let sent = String::from_utf8(terminal.sent()).unwrap();
    let at = format!("{x};{y}");
    assert_eq!(sent, format!("\x1b[<0;{at}M\x1b[<0;{at}m\x1b[<2;{at}M\x1b[<2;{at}m\x1b[<64;{at}M\x1b[<64;{at}M"));
    assert_eq!(terminal.selection_text(), None);
    assert_eq!(terminal.screen().display_offset, 0, "the wheel went to the program, not the history");
}

#[gpui_kit::test]
fn with_the_kitty_event_types_on_a_held_key_repeats_and_comes_up(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke};
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // disambiguate (1) and event types (2)
    terminal.feed(b"\x1b[>3u");
    let before = terminal.sent().len();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let ctrl_c = Keystroke::parse("ctrl-c").unwrap();
        for held in [false, true] {
            let down = KeyDownEvent { keystroke: ctrl_c.clone(), is_held: held, prefer_character_input: false };
            window.dispatch_event(down.to_platform_input(), cx);
        }
        window.dispatch_event(KeyUpEvent { keystroke: ctrl_c }.to_platform_input(), cx);
    })
    .unwrap();
    assert_eq!(&terminal.sent()[before..], b"\x1b[99;5u\x1b[99;5:2u\x1b[99;5:3u");
}

#[gpui_kit::test]
fn a_program_that_asks_for_moves_hears_drags_and_hovers_once_per_cell(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // moves while a button is held (1002), the SGR way
    terminal.feed(b"\x1b[?1002h\x1b[?1006h");
    let layout = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn")
        })
        .unwrap();
    let centre = |row: usize, col: usize| {
        point(
            layout.origin.x + layout.cell_width * (col as f32 + 0.5),
            layout.origin.y + layout.line_height * (row as f32 + 0.5),
        )
    };
    cx.update_window(window, |_, window, cx| window.drag(centre(2, 3), centre(2, 9), cx)).unwrap();
    let sent = String::from_utf8(terminal.sent()).unwrap();
    let reports: Vec<&str> = sent.split('\x1b').filter(|r| !r.is_empty()).collect();
    assert_eq!(reports.first(), Some(&"[<0;4;3M"), "{sent:?}");
    assert_eq!(reports.last(), Some(&"[<0;10;3m"), "{sent:?}");
    let moves: Vec<usize> = reports[1..reports.len() - 1]
        .iter()
        .map(|r| r.strip_prefix("[<32;").and_then(|r| r.strip_suffix(";3M")).expect(r).parse().unwrap())
        .collect();
    assert!(moves.windows(2).all(|w| w[0] < w[1]), "each cell once, left to right: {moves:?}");
    assert_eq!(moves.last(), Some(&10));
    assert_eq!(terminal.selection_text(), None, "the program had the mouse");
    // every move (1003): a hover is reported too, as button 3 + 32
    terminal.feed(b"\x1b[?1003h");
    let before = terminal.sent().len();
    cx.update_window(window, |_, window, cx| window.hover("terminal", cx)).unwrap();
    let hover = String::from_utf8(terminal.sent()[before..].to_vec()).unwrap();
    assert!(hover.starts_with("\x1b[<35;") && hover.ends_with('M'), "{hover:?}");
}

#[gpui_kit::test]
fn files_dropped_from_finder_go_in_as_their_paths(cx: &mut TestAppContext) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[?2004h");
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("terminal").bounds().center();
        let paths =
            ExternalPaths(vec![std::path::PathBuf::from("/Users/h/Desktop/shot 1.png"), "/tmp/notes.md".into()].into());
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
        window.render_frame(cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
    })
    .unwrap();
    assert_eq!(terminal.sent(), "\x1b[200~/Users/h/Desktop/shot\\ 1.png /tmp/notes.md\x1b[201~".as_bytes());
}

/// Where the middle of a cell is, as an offset from the terminal's top left corner.
fn cell_offset(
    window: &mut gpui_kit::Window,
    shell: &Entity<Shell>,
    col: usize,
    row: usize,
    cx: &mut gpui_kit::App,
) -> gpui_kit::Point<gpui_kit::Pixels> {
    window.render_frame(cx);
    let layout = shell.read(cx).terminal().read(cx).layout().expect("drawn");
    let corner = window.find("terminal").bounds().origin;
    point(
        layout.origin.x - corner.x + layout.cell_width * (col as f32 + 0.5),
        layout.origin.y - corner.y + layout.line_height * (row as f32 + 0.5),
    )
}

#[gpui_kit::test]
fn a_double_click_selects_a_word_and_a_triple_click_its_line(cx: &mut TestAppContext) {
    use gpui_kit::test::ClickOptions;
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[2J\x1b[Hhello world\r\n");
    cx.update_window(window, |_, window, cx| {
        let at = cell_offset(window, &shell, 8, 0, cx);
        window.click_with_options("terminal", ClickOptions::new().with_offset(at).with_count(2), cx);
    })
    .unwrap();
    assert_eq!(terminal.selection_text().as_deref(), Some("world"));
    cx.update_window(window, |_, window, cx| {
        let at = cell_offset(window, &shell, 2, 0, cx);
        window.click_with_options("terminal", ClickOptions::new().with_offset(at).with_count(3), cx);
    })
    .unwrap();
    assert_eq!(terminal.selection_text().as_deref(), Some("hello world"));
}

#[gpui_kit::test]
fn cmd_shows_a_link_under_the_pointer_and_cmd_click_opens_it(cx: &mut TestAppContext) {
    use gpui_kit::test::ClickOptions;
    use gpui_kit::{InputEvent as _, Modifiers, ModifiersChangedEvent, MouseMoveEvent};
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[2J\x1b[Hsee https://example.com/x now\r\n");
    let hovered = |cx: &mut TestAppContext| {
        shell.read_with(cx, |shell, cx| shell.terminal().read(cx).hovered_link().map(|link| link.url.clone()))
    };
    cx.update_window(window, |_, window, cx| {
        let corner = window.find("terminal").bounds().origin;
        let position = corner + cell_offset(window, &shell, 9, 0, cx);
        let moved = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::command() };
        window.dispatch_event(moved.to_platform_input(), cx);
    })
    .unwrap();
    assert_eq!(hovered(cx).as_deref(), Some("https://example.com/x"), "⌘ over the address marks it");
    cx.update_window(window, |_, window, cx| {
        let up = ModifiersChangedEvent { modifiers: Modifiers::default(), ..Default::default() };
        window.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    assert_eq!(hovered(cx), None, "letting go of ⌘ clears it");
    assert_eq!(cx.opened_url(), None);
    cx.update_window(window, |_, window, cx| {
        let at = cell_offset(window, &shell, 9, 0, cx);
        let options = ClickOptions::new().with_offset(at).with_modifiers(Modifiers::command());
        window.click_with_options("terminal", options, cx);
    })
    .unwrap();
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/x"));
    assert_eq!(terminal.selection_text(), None, "the click opened the link, it selected nothing");
    assert!(terminal.sent().is_empty());
}

#[gpui_kit::test]
fn scrolled_back_the_view_stays_on_its_lines_while_output_arrives(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    for i in 0..300 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    let line_height = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn").line_height
        })
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.scroll("terminal", ScrollDelta::Pixels(point(px(0.), line_height * 40.)), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let before = terminal.screen();
    assert_eq!(before.display_offset, 40);
    for i in 300..350 {
        terminal.feed(format!("more {i}\r\n").as_bytes());
    }
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let after = terminal.screen();
    assert_eq!(after.text(), before.text(), "the lines you were reading stay put");
    assert_eq!(after.display_offset, 90);
}

#[gpui_kit::test]
fn the_cursor_blinks_while_the_terminal_has_the_keyboard(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let shown = |cx: &mut TestAppContext| shell.read_with(cx, |shell, cx| shell.terminal().read(cx).cursor_shown());
    let half = std::time::Duration::from_millis(600);
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.update_window(window, |_, window, _| window.activate_window()).unwrap();
    cx.run_until_parked();
    assert!(shown(cx));
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert!(!shown(cx), "off for a moment");
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert!(shown(cx), "and back");
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert!(!shown(cx));
    cx.update_window(window, |_, window, cx| window.press("a", cx)).unwrap();
    cx.run_until_parked();
    assert!(shown(cx), "typing shows it at once");
    // a program that asks for a steady cursor
    let terminal = terminal_of(&shell, cx);
    terminal.feed(b"\x1b[2 q");
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert!(shown(cx), "a steady cursor stays");
    terminal.feed(b"\x1b[0 q");
    // the palette takes the keyboard
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-k", cx);
        window.render_frame(cx);
    })
    .unwrap();
    for _ in 0..3 {
        cx.executor().advance_clock(half);
        cx.run_until_parked();
        assert!(shown(cx), "without the keyboard it stays, drawn hollow");
    }
}

#[gpui_kit::test]
fn an_input_methods_composition_shows_at_the_cursor_and_only_what_it_commits_is_typed(cx: &mut TestAppContext) {
    use gpui_kit::{Bounds, EntityInputHandler as _};
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    // a shell prompt, nine cells wide, with the cursor after it
    terminal.feed(b"\x1b[2J\x1b[H~/shop % ");
    let view = shell.read_with(cx, |shell, _| shell.terminal().clone());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "にほ", Some(2..2), window, cx);
            assert_eq!(view.marked_text_range(window, cx), Some(0..2));
            assert_eq!(view.marked_text(), Some("にほ"));
            // the candidates go under the composition's caret: two wide letters past the cursor's cell
            let layout = view.layout().expect("drawn");
            let at = view.bounds_for_range(2..2, Bounds::default(), window, cx).expect("a place");
            assert_eq!(at.origin, point(layout.origin.x + layout.cell_width * 13., layout.origin.y));
        });
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(terminal.sent(), b"", "nothing is typed while composing");
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| view.replace_text_in_range(None, "日本", window, cx));
    })
    .unwrap();
    assert_eq!(terminal.sent(), "日本".as_bytes());
    assert_eq!(view.read_with(cx, |view, _| view.marked_text().map(String::from)), None);
}

#[gpui_kit::test]
fn a_frame_shapes_only_the_rows_that_changed(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    let frame = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        shell.read_with(cx, |shell, cx| shell.terminal().read(cx).rows_shaped())
    };
    frame(cx);
    terminal.feed(b"\x1b[2J\x1b[Hline 0\r\nline 1\r\nline 2\r\nline 3\r\nline 4");
    assert_eq!(frame(cx), 5, "five new rows");
    assert_eq!(frame(cx), 0, "nothing new: nothing shaped");
    terminal.feed(b" and more");
    assert_eq!(frame(cx), 1, "one row changed");
    // a full screen, then one line more: everything moves up a row, and only the new line is shaped
    let rows = shell.read_with(cx, |shell, cx| shell.terminal().read(cx).layout().expect("drawn").rows);
    for i in 0..rows + 3 {
        terminal.feed(format!("\r\nrow {i}").as_bytes());
    }
    frame(cx);
    terminal.feed(format!("\r\nrow {}", rows + 3).as_bytes());
    assert_eq!(frame(cx), 1, "a scroll shapes only the new line");
    // the cursor blinking shapes nothing
    cx.update_window(window, |_, window, _| window.activate_window()).unwrap();
    frame(cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(frame(cx), 0);
}

#[gpui_kit::test]
fn a_selection_dragged_past_the_top_edge_scrolls_the_history_and_keeps_selecting(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    let (window, shell) = open(cx);
    let terminal = terminal_of(&shell, cx);
    for i in 0..300 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    let layout = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            shell.read(cx).terminal().read(cx).layout().expect("drawn")
        })
        .unwrap();
    let inside = point(layout.origin.x + layout.cell_width * 3.5, layout.origin.y + layout.line_height * 5.5);
    // above the terminal, in the island's band
    let above = point(inside.x, layout.origin.y - px(30.));
    let left = MouseButton::Left;
    let modifiers = Modifiers::default();
    cx.update_window(window, |_, window, cx| {
        let down = MouseDownEvent { button: left, position: inside, modifiers, click_count: 1, first_mouse: false };
        window.dispatch_event(down.to_platform_input(), cx);
        window.render_frame(cx);
        let moved = MouseMoveEvent { position: above, pressed_button: Some(left), modifiers };
        window.dispatch_event(moved.to_platform_input(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let tick = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    };
    for _ in 0..5 {
        tick(cx);
    }
    let back = terminal.screen().display_offset;
    assert!(back >= 5, "the history scrolled while the pointer was past the edge: {back}");
    let lines = terminal.selection_text().expect("a selection").lines().count();
    assert_eq!(lines, 6 + back, "the six rows it began with and every line scrolled back");
    // the button comes up out there: the selection stands, and nothing scrolls any more
    cx.update_window(window, |_, window, cx| {
        let up = MouseUpEvent { button: left, position: above, modifiers, click_count: 1 };
        window.dispatch_event(up.to_platform_input(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    for _ in 0..3 {
        tick(cx);
    }
    assert_eq!(terminal.screen().display_offset, back);
    assert_eq!(terminal.selection_text().expect("still selected").lines().count(), lines);
}

/// Item 4.6: every button that had no tooltip has one (`Tip`). The keys beside them are the library's own drawing of
/// the action's key (looked at in the pictures).
#[gpui_kit::test]
fn the_top_rows_buttons_and_the_terminals_plus_say_what_they_do_when_pointed_at(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    let tip = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            // from somewhere else first: a tooltip comes when the pointer arrives
            w.hover("terminal", cx);
            w.hover(id, cx);
        })
        .unwrap();
        cx.executor().advance_clock(std::time::Duration::from_millis(900));
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("tip").and_then(|tip| tip.label().map(String::from))
        })
        .unwrap()
    };
    assert_eq!(tip(cx, "add-project").as_deref(), Some("Add project"));
    assert_eq!(tip(cx, "palette").as_deref(), Some("Commands"));
    assert_eq!(tip(cx, "settings").as_deref(), Some("Settings"));
    assert_eq!(tip(cx, "new-workspace").as_deref(), Some("New workspace in shop"));
    assert_eq!(tip(cx, "ws-menu").as_deref(), Some("Workspace actions"));
    assert_eq!(tip(cx, "term-add").as_deref(), Some("New terminal"));
    // a menu's trigger says nothing while its menu is open
    cx.update_window(window, |_, w, cx| {
        w.hover("terminal", cx);
        w.click("ws-menu", cx);
    })
    .unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("remove-workspace").is_some(), "the menu opened");
        assert!(w.try_find("tip").is_none(), "no tooltip over an open menu");
    })
    .unwrap();
}

/// Item 4.6: how long a toast stays. What went wrong stays until it is closed; news goes after 4 seconds, or 8 with
/// something under it to read; a long detail is folded to three lines until "Show all".
#[gpui_kit::test]
fn a_toast_stays_as_long_as_it_takes_to_read_and_what_went_wrong_until_it_is_closed(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    let titles = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            let mut said: Vec<String> =
                w.find_all("toast").iter().map(|toast| toast.label().unwrap_or_default().to_string()).collect();
            said.sort();
            said
        })
        .unwrap()
    };
    let wait = |cx: &mut TestAppContext, seconds: f32| {
        cx.executor().advance_clock(std::time::Duration::from_secs_f32(seconds));
        cx.run_until_parked();
        // its leaving takes a moment more
        cx.executor().advance_clock(std::time::Duration::from_millis(600));
    };
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        enkidu_app::toast::good(w, "Settings saved", cx);
        enkidu_app::toast::note(w, "Path copied", "~/code/shop", cx);
        enkidu_app::toast::say(w, "The project could not be added.", cx);
        let detail = "Copied 9 files with its changes into ~/code/shop; new changes follow while it is Live.\n\n$ on \
                      (stock-fix)\nrestarting web\nrestarting worker\ndone";
        enkidu_app::toast::done(w, "stock-fix is Live", detail, cx);
    })
    .unwrap();
    assert_eq!(titles(cx), ["Path copied", "Settings saved", "The project could not be added.", "stock-fix is Live"]);
    // (they have risen into place)
    cx.executor().advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    // the long one is folded, and unfolds
    let unfold = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.find("toast-unfold").label().map(String::from)
        })
        .unwrap()
    };
    assert_eq!(unfold(cx).as_deref(), Some("Show all"), "only the long one has it, or this would be ambiguous");
    let height = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            let details = w.find_all("toast-detail");
            let long =
                details.iter().find(|detail| detail.label().is_some_and(|said| said.starts_with("Copied 9 files")));
            long.expect("the long detail").bounds().size.height
        })
        .unwrap()
    };
    let folded = height(cx);
    cx.update_window(window, |_, w, cx| w.click("toast-unfold", cx)).unwrap();
    assert_eq!(unfold(cx).as_deref(), Some("Show less"));
    let whole = height(cx);
    assert!(whole > folded, "unfolded it is taller: {folded:?} then {whole:?}");

    // after 4 seconds the one-line news is gone; after 8 the ones with a detail; what went wrong stays
    wait(cx, 3.5);
    assert_eq!(titles(cx), ["Path copied", "The project could not be added.", "stock-fix is Live"]);
    wait(cx, 4.0);
    assert_eq!(titles(cx), ["The project could not be added."]);
    wait(cx, 30.);
    assert_eq!(titles(cx), ["The project could not be added."], "what went wrong stays until it is closed");
}
