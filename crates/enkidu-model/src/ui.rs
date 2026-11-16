//! What the window remembers of how it is laid out (DESIGN.md, Principle 8: "the diff layout... survive a restart"),
//! and the plain logic of the workspace's layouts. It grows with the screens: each item adds what it needs kept.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// How a diff shows a file's two versions.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffLayout {
    /// One column: the removed lines, then the added ones.
    Unified,
    /// Before and after, side by side.
    #[default]
    Split,
}

/// Where the changes go when they share the workspace with Claude's terminal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// Beside the terminal: the terminal on the left, the changes on the right.
    Side,
    /// Above the terminal (the default).
    #[default]
    Stack,
}

/// A panel at the right edge of the workspace (DESIGN.md: "The right edge is a tool bar").
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Panel {
    /// Changed files: every changed file, then the files that have remarks only.
    Changes,
    /// Files: the workspace as git sees it.
    Files,
}

/// What a workspace shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The changes beside the terminal.
    Side,
    /// The changes above the terminal.
    Stack,
    /// The terminal alone.
    Terminal,
    /// The changes alone (the terminal keeps running behind them).
    Review,
}

/// The width of the changes beside the terminal until a drag says otherwise, in px.
pub const REVIEW_WIDTH: u32 = 540;
/// The height of the terminal under the changes until a drag says otherwise.
pub const TERMINAL_HEIGHT: u32 = 320;
/// The least each island keeps when the space between two is dragged.
pub const MIN_REVIEW_WIDTH: f32 = 340.;
pub const MIN_TERMINAL_WIDTH: f32 = 360.;
pub const MIN_TERMINAL_HEIGHT: f32 = 140.;
pub const MIN_CHANGES_HEIGHT: f32 = 120.;
/// The space between two islands, which is also where they are resized.
pub const GAP: f32 = 8.;
/// The width of the panel at the right edge until a drag says otherwise, and what a drag keeps it within.
pub const PANEL_WIDTH: u32 = 280;
pub const MIN_PANEL_WIDTH: f32 = 200.;
pub const MAX_PANEL_WIDTH: f32 = 520.;
/// The width of the bar at the right edge that opens the panels.
pub const RAIL_WIDTH: f32 = 40.;

/// The colors code is drawn in, in a diff and in an opened file (item 8.3).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodeColors {
    /// An IDE's, PyCharm's dark scheme: keywords, strings, numbers and definitions each in their own color.
    #[default]
    Ide,
    /// The quiet one: mostly white, with soft peach and mint.
    Quiet,
}

/// The size of code in a diff and in an opened file until you change it, in px, and how far it goes (item 8.4).
pub const CODE_SIZE: u32 = 12;
pub const MIN_CODE_SIZE: u32 = 9;
pub const MAX_CODE_SIZE: u32 = 24;
/// The same for the terminal's text (12.5 px in DESIGN.md's "Type").
pub const TERMINAL_SIZE: f32 = 12.5;
pub const MIN_TERMINAL_SIZE: f32 = 8.5;
pub const MAX_TERMINAL_SIZE: f32 = 24.5;

/// The height of a line of code of that size, in px: 12 px code sits on 20 (DESIGN.md, "Type"), and every other
/// size keeps that share.
pub fn code_row_height(size: u32) -> u32 {
    (size * 5 + 1) / 3
}

/// The layout as it was left.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ui {
    pub diff_layout: DiffLayout,
    /// The changes beside the terminal, or above it.
    pub layout: Layout,
    /// The width of the changes beside the terminal, in px.
    pub review_width: u32,
    /// The height of the terminal under the changes, in px.
    pub terminal_height: u32,
    /// The changes have the whole workspace.
    pub review_maximized: bool,
    /// The changes are put away: the terminal has the whole workspace.
    pub review_hidden: bool,
    /// The panel open at the right edge, if one is.
    pub right_panel: Option<Panel>,
    /// Its width, in px.
    pub right_panel_width: u32,
    /// On the Work page: only the cards with something waiting on you (whether the page itself is on screen is kept
    /// with where you are, `frame::View`).
    pub work_needs_you: bool,
    /// The size of code in a diff and in an opened file, in px.
    pub code_size: u32,
    /// The colors that code is drawn in.
    pub code_colors: CodeColors,
    /// The size of the terminal's text, in px.
    pub terminal_size: f32,
    /// Setup was shown on this Mac, or there was a project already: the first launch's welcome is behind you
    /// (item 21.6).
    pub setup_seen: bool,
}

impl Default for Ui {
    fn default() -> Ui {
        Ui {
            diff_layout: DiffLayout::default(),
            layout: Layout::default(),
            review_width: REVIEW_WIDTH,
            terminal_height: TERMINAL_HEIGHT,
            review_maximized: false,
            review_hidden: false,
            // the changed files are listed as the app first starts
            right_panel: Some(Panel::Changes),
            right_panel_width: PANEL_WIDTH,
            work_needs_you: false,
            code_size: CODE_SIZE,
            code_colors: CodeColors::default(),
            terminal_size: TERMINAL_SIZE,
            setup_seen: false,
        }
    }
}

impl Ui {
    /// The layout kept in `text` (the app's `ui.json`). Each setting is read on its own: one an older version didn't
    /// write, or one that can't be read, is its default and the others stay as they were left. What can't be read at
    /// all is the default layout.
    pub fn read(text: &str) -> Ui {
        let Ok(serde_json::Value::Object(saved)) = serde_json::from_str::<serde_json::Value>(text) else {
            return Ui::default();
        };
        fn kept<T: DeserializeOwned>(saved: &serde_json::Map<String, serde_json::Value>, key: &str, default: T) -> T {
            saved.get(key).and_then(|value| serde_json::from_value(value.clone()).ok()).unwrap_or(default)
        }
        let default = Ui::default();
        Ui {
            diff_layout: kept(&saved, "diffLayout", default.diff_layout),
            layout: kept(&saved, "layout", default.layout),
            review_width: kept(&saved, "reviewWidth", default.review_width),
            terminal_height: kept(&saved, "terminalHeight", default.terminal_height),
            review_maximized: kept(&saved, "reviewMaximized", default.review_maximized),
            review_hidden: kept(&saved, "reviewHidden", default.review_hidden),
            // a closed panel is kept as null, which is not the same as nothing kept
            right_panel: kept(&saved, "rightPanel", default.right_panel),
            right_panel_width: kept(&saved, "rightPanelWidth", default.right_panel_width),
            work_needs_you: kept(&saved, "workNeedsYou", default.work_needs_you),
            // a size from outside what the keys and Settings give (a file edited by hand) is brought back inside
            code_size: kept(&saved, "codeSize", default.code_size).clamp(MIN_CODE_SIZE, MAX_CODE_SIZE),
            code_colors: kept(&saved, "codeColors", default.code_colors),
            terminal_size: {
                let size: f32 = kept(&saved, "terminalSize", default.terminal_size);
                if size.is_finite() { size.clamp(MIN_TERMINAL_SIZE, MAX_TERMINAL_SIZE) } else { default.terminal_size }
            },
            setup_seen: kept(&saved, "setupSeen", default.setup_seen),
        }
    }

    pub fn write(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// What was chosen, as the switch in the tabs row shows it.
    pub fn chosen(&self) -> Mode {
        match (self.review_maximized, self.review_hidden, self.layout) {
            (true, _, _) => Mode::Review,
            (false, true, _) => Mode::Terminal,
            (false, false, Layout::Side) => Mode::Side,
            (false, false, Layout::Stack) => Mode::Stack,
        }
    }

    /// What the workspace shows. With nothing to review (`quiet`: no changes, no remarks, no file open) Claude's
    /// terminal has the whole workspace, whatever was chosen; the first change brings the changes back where the
    /// layout puts them.
    pub fn mode(&self, quiet: bool) -> Mode {
        if quiet { Mode::Terminal } else { self.chosen() }
    }

    /// The switch: the changes beside the terminal, or above it.
    pub fn show(&mut self, layout: Layout) {
        (self.layout, self.review_hidden, self.review_maximized) = (layout, false, false);
    }

    /// The switch's third choice: the terminal alone.
    pub fn terminal_only(&mut self) {
        (self.review_hidden, self.review_maximized) = (true, false);
    }

    /// ⌥⌘L: the changes beside the terminal, or back above it.
    pub fn toggle_side(&mut self) {
        self.show(match self.layout {
            Layout::Stack => Layout::Side,
            Layout::Side => Layout::Stack,
        });
    }

    /// ⌥⌘B: the terminal alone, or the changes back.
    pub fn toggle_terminal_only(&mut self) {
        (self.review_hidden, self.review_maximized) = (!self.review_hidden, false);
    }

    /// ⇧⌘E: the changes have the whole workspace, or the terminal is back.
    pub fn toggle_full_review(&mut self) {
        (self.review_maximized, self.review_hidden) = (!self.review_maximized, false);
    }

    /// A button of the right edge, ⌥⌘1 or ⌥⌘2: its panel opens beside the bar, or closes if it was the one open.
    pub fn toggle_panel(&mut self, panel: Panel) {
        self.right_panel = if self.right_panel == Some(panel) { None } else { Some(panel) };
    }

    /// The changes must be on screen: a file was opened where they are, or a file's changes were asked for. False
    /// when they already were.
    pub fn reveal_review(&mut self) -> bool {
        std::mem::take(&mut self.review_hidden)
    }

    /// Claude's terminal must be on screen: remarks were typed into its prompt, and Enter is pressed there. False
    /// when it already was.
    pub fn reveal_terminal(&mut self) -> bool {
        std::mem::take(&mut self.review_maximized)
    }

    /// ⌘+ or ⌘− while the changes have the keyboard: the code one size up or down, between 9 and 24 px. False when
    /// it was at that end already.
    pub fn step_code_size(&mut self, up: bool) -> bool {
        let size = if up { self.code_size + 1 } else { self.code_size.saturating_sub(1) };
        self.set_code_size(size)
    }

    /// The code at this size, kept between 9 and 24 px. False when it was that size already.
    pub fn set_code_size(&mut self, size: u32) -> bool {
        let size = size.clamp(MIN_CODE_SIZE, MAX_CODE_SIZE);
        std::mem::replace(&mut self.code_size, size) != size
    }

    /// ⌘+ or ⌘− while a terminal has the keyboard: its text one size up or down, between 8.5 and 24.5 px. False when
    /// it was at that end already.
    pub fn step_terminal_size(&mut self, up: bool) -> bool {
        self.set_terminal_size(self.terminal_size + if up { 1. } else { -1. })
    }

    /// The terminal's text at this size, kept between 8.5 and 24.5 px. False when it was that size already.
    pub fn set_terminal_size(&mut self, size: f32) -> bool {
        let size = if size.is_finite() { size.clamp(MIN_TERMINAL_SIZE, MAX_TERMINAL_SIZE) } else { TERMINAL_SIZE };
        std::mem::replace(&mut self.terminal_size, size) != size
    }
}

/// The width the changes get when the space between them and the terminal is dragged: the pointer is `from_right`
/// px from the right edge of the two islands, which are `room` px wide together with the gap between them. The
/// changes keep 340 px, and the terminal 360.
pub fn dragged_review_width(from_right: f32, room: f32) -> u32 {
    let width = (from_right - GAP / 2.).max(MIN_REVIEW_WIDTH).min(room - GAP - MIN_TERMINAL_WIDTH);
    width.round().max(0.) as u32
}

/// The height the terminal gets when the space between it and the changes above it is dragged: the pointer is
/// `from_bottom` px from the bottom edge of the two islands, which are `room` px tall together with the gap between
/// them. The terminal keeps 140 px, and the changes 120.
pub fn dragged_terminal_height(from_bottom: f32, room: f32) -> u32 {
    let height = (from_bottom - GAP / 2.).max(MIN_TERMINAL_HEIGHT).min(room - GAP - MIN_CHANGES_HEIGHT);
    height.round().max(0.) as u32
}

/// The width the panel at the right edge gets when the space beside it is dragged: the pointer is `from_right` px
/// from the right edge of the workspace, bar included. Between 200 and 520 px.
pub fn dragged_panel_width(from_right: f32) -> u32 {
    (from_right - RAIL_WIDTH - GAP / 2.).clamp(MIN_PANEL_WIDTH, MAX_PANEL_WIDTH).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_is_kept_and_what_cant_be_read_is_the_default() {
        assert_eq!(Ui::default().diff_layout, DiffLayout::Split, "side by side");
        let one_column = Ui { diff_layout: DiffLayout::Unified, ..Ui::default() };
        assert!(one_column.write().starts_with(r#"{"diffLayout":"unified","#), "{}", one_column.write());
        assert_eq!(Ui::read(&one_column.write()), one_column);
        // a file from before a setting existed, one with settings from a later version, and a broken one
        assert_eq!(Ui::read("{}"), Ui::default());
        assert_eq!(Ui::read(r#"{"diffLayout":"unified"}"#), one_column, "a file with that one setting");
        assert_eq!(Ui::read(r#"{"diffLayout":"unified","somethingNewer":1}"#), one_column);
        assert_eq!(Ui::read(r#"{"diffLayout":"sideways"}"#), Ui::default());
        assert_eq!(Ui::read("not json"), Ui::default());
        assert_eq!(Ui::read("[1, 2]"), Ui::default());
    }

    /// Remembers the layout across restarts.
    #[test]
    fn remembers_the_layout_across_restarts() {
        let default = Ui::default();
        assert_eq!((default.layout, default.review_width, default.terminal_height), (Layout::Stack, 540, 320));
        assert!(!default.review_maximized && !default.review_hidden);
        let left = Ui {
            diff_layout: DiffLayout::Split,
            layout: Layout::Side,
            review_width: 612,
            terminal_height: 280,
            review_maximized: false,
            review_hidden: true,
            right_panel: Some(Panel::Files),
            right_panel_width: 300,
            work_needs_you: true,
            code_size: 14,
            code_colors: CodeColors::Quiet,
            terminal_size: 13.5,
            setup_seen: true,
        };
        assert_eq!(
            left.write(),
            concat!(
                r#"{"diffLayout":"split","layout":"side","reviewWidth":612,"terminalHeight":280,"#,
                r#""reviewMaximized":false,"reviewHidden":true,"rightPanel":"files","rightPanelWidth":300,"#,
                r#""workNeedsYou":true,"codeSize":14,"codeColors":"quiet","terminalSize":13.5,"setupSeen":true}"#
            )
        );
        // a panel that was closed stays closed, and a file from before there were panels opens the changed files
        let closed = Ui { right_panel: None, ..Ui::default() };
        assert!(closed.write().contains(r#""rightPanel":null"#), "{}", closed.write());
        assert_eq!(Ui::read(&closed.write()).right_panel, None);
        assert_eq!(Ui::read(r#"{"layout":"side"}"#).right_panel, Some(Panel::Changes));
        assert_eq!(Ui::read(r#"{"rightPanel":"sideways"}"#).right_panel, Some(Panel::Changes));
        assert_eq!(Ui::read(&left.write()), left);
        // a file from before the Work page: every card is shown
        assert!(!Ui::read(r#"{"layout":"side"}"#).work_needs_you);
        // a file from before Setup (item 21.6): it was never shown here, so a Mac without a project still gets it
        assert!(!Ui::read(r#"{"layout":"side"}"#).setup_seen);
        // one setting that can't be read is its default; the others stay as they were left
        let part = Ui::read(r#"{"layout":"diagonal","reviewWidth":"wide","terminalHeight":280,"reviewHidden":true}"#);
        assert_eq!((part.layout, part.review_width, part.terminal_height), (Layout::Stack, 540, 280));
        assert!(part.review_hidden);
    }

    /// Reads layout prefs saved by older versions without breaking.
    #[test]
    fn reads_layout_prefs_saved_by_older_versions_without_breaking() {
        // older versions saved sidebar settings (the sidebar is gone); a broken value falls back to its default
        // instead of reaching the screen
        let old = concat!(
            r#"{"sidebarCollapsed":true,"sidebarWidth":280,"sidebarAutoHide":false,"#,
            r#""layout":"diagonal","terminalHeight":"big","reviewWidth":600}"#
        );
        let ui = Ui::read(old);
        assert_eq!(ui.review_width, 600);
        assert_eq!(ui.layout, Ui::default().layout);
        assert_eq!(ui.terminal_height, Ui::default().terminal_height);
        // what it doesn't know isn't kept
        let written = ui.write();
        assert!(!written.contains("sidebar"), "{written}");
        assert_eq!(Ui::read("not json"), Ui::default());
    }

    #[test]
    fn nothing_to_review_gives_the_terminal_the_workspace_whatever_was_chosen() {
        let mut ui = Ui::default();
        assert_eq!((ui.chosen(), ui.mode(false), ui.mode(true)), (Mode::Stack, Mode::Stack, Mode::Terminal));
        ui.show(Layout::Side);
        assert_eq!((ui.chosen(), ui.mode(false), ui.mode(true)), (Mode::Side, Mode::Side, Mode::Terminal));
        ui.toggle_full_review();
        assert_eq!((ui.chosen(), ui.mode(false), ui.mode(true)), (Mode::Review, Mode::Review, Mode::Terminal));
    }

    #[test]
    fn the_keys_and_the_switch_go_between_the_layouts() {
        let mut ui = Ui::default();
        // ⌥⌘L: beside, and back above
        ui.toggle_side();
        assert_eq!(ui.chosen(), Mode::Side);
        ui.toggle_side();
        assert_eq!(ui.chosen(), Mode::Stack);
        // ⌥⌘B: the terminal alone, and the changes back where they were
        ui.toggle_side();
        ui.toggle_terminal_only();
        assert_eq!(ui.chosen(), Mode::Terminal);
        ui.toggle_terminal_only();
        assert_eq!(ui.chosen(), Mode::Side);
        // ⇧⌘E: the changes alone, and back
        ui.toggle_full_review();
        assert_eq!(ui.chosen(), Mode::Review);
        ui.toggle_full_review();
        assert_eq!(ui.chosen(), Mode::Side);
        // each key leaves the other's view: ⌥⌘B from the full review is the terminal alone, ⇧⌘E from there the review
        ui.toggle_full_review();
        ui.toggle_terminal_only();
        assert_eq!(ui.chosen(), Mode::Terminal);
        ui.toggle_full_review();
        assert_eq!(ui.chosen(), Mode::Review);
        // ⌥⌘L from either brings both islands back
        ui.toggle_side();
        assert_eq!(ui.chosen(), Mode::Stack);
        // the switch
        ui.terminal_only();
        assert_eq!(ui.chosen(), Mode::Terminal);
        ui.show(Layout::Stack);
        assert_eq!(ui.chosen(), Mode::Stack);
        // what must be on screen comes back, and says whether anything changed
        ui.terminal_only();
        assert!(ui.reveal_review() && !ui.reveal_review());
        assert_eq!(ui.chosen(), Mode::Stack);
        ui.toggle_full_review();
        assert!(ui.reveal_terminal() && !ui.reveal_terminal());
        assert_eq!(ui.chosen(), Mode::Stack);
    }

    #[test]
    fn the_right_edge_opens_one_panel_at_a_time() {
        let mut ui = Ui::default();
        assert_eq!((ui.right_panel, ui.right_panel_width), (Some(Panel::Changes), 280));
        // the other button: its panel takes the place
        ui.toggle_panel(Panel::Files);
        assert_eq!(ui.right_panel, Some(Panel::Files));
        // the same button again: closed
        ui.toggle_panel(Panel::Files);
        assert_eq!(ui.right_panel, None);
        ui.toggle_panel(Panel::Changes);
        assert_eq!(ui.right_panel, Some(Panel::Changes));
        // dragged: the pointer 324 px from the window's edge gives 280 (the bar is 40, half the gap 4)
        assert_eq!(dragged_panel_width(324.), 280);
        assert_eq!(dragged_panel_width(50.), 200, "200 px at least");
        assert_eq!(dragged_panel_width(900.), 520, "520 px at most");
    }

    #[test]
    fn a_dragged_gap_leaves_each_island_its_least_size() {
        // beside: the pointer 544 px from the right edge of 1200 px gives the changes 540
        assert_eq!(dragged_review_width(544., 1200.), 540);
        // the changes keep 340 px, the terminal 360
        assert_eq!(dragged_review_width(100., 1200.), 340);
        assert_eq!(dragged_review_width(1190., 1200.), 832, "1200 less the gap and the terminal's 360");
        // above: the pointer 324 px from the bottom edge of 800 px gives the terminal 320
        assert_eq!(dragged_terminal_height(324., 800.), 320);
        // the terminal keeps 140 px, the changes 120
        assert_eq!(dragged_terminal_height(20., 800.), 140);
        assert_eq!(dragged_terminal_height(790., 800.), 672, "800 less the gap and the changes' 120");
        // halves round to whole pixels
        assert_eq!(dragged_review_width(400.6, 1200.), 397);
        // a window too small for both leasts: the terminal's comes first, and nothing goes under zero
        assert_eq!(dragged_review_width(500., 600.), 232);
        assert_eq!(dragged_review_width(500., 300.), 0);
    }

    /// Item 8.4: the code's size, and the terminal's.
    #[test]
    fn the_sizes_of_code_and_of_the_terminal_step_stay_in_bounds_and_are_kept() {
        let mut ui = Ui::default();
        assert_eq!((ui.code_size, ui.terminal_size, ui.code_colors), (12, 12.5, CodeColors::Ide));
        // 12 px code sits on 20 px lines, and the other sizes keep that share
        let rows: Vec<u32> = [9, 10, 11, 12, 13, 14, 16, 18, 24].into_iter().map(code_row_height).collect();
        assert_eq!(rows, [15, 17, 18, 20, 22, 23, 27, 30, 40]);
        assert!(ui.step_code_size(true) && ui.step_code_size(true));
        assert_eq!(ui.code_size, 14);
        assert!(ui.step_code_size(false));
        assert_eq!(ui.code_size, 13);
        // the ends: nothing changes there, and the caller hears it
        for _ in 0..30 {
            ui.step_code_size(true);
        }
        assert_eq!(ui.code_size, MAX_CODE_SIZE);
        assert!(!ui.step_code_size(true));
        for _ in 0..30 {
            ui.step_code_size(false);
        }
        assert_eq!(ui.code_size, MIN_CODE_SIZE);
        assert!(!ui.step_code_size(false));
        assert!(ui.set_code_size(CODE_SIZE) && !ui.set_code_size(CODE_SIZE));
        assert!(ui.set_code_size(400));
        assert_eq!(ui.code_size, MAX_CODE_SIZE);
        // the terminal keeps its half px
        assert!(ui.step_terminal_size(true));
        assert_eq!(ui.terminal_size, 13.5);
        assert!(ui.step_terminal_size(false) && ui.step_terminal_size(false));
        assert_eq!(ui.terminal_size, 11.5);
        for _ in 0..30 {
            ui.step_terminal_size(true);
        }
        assert_eq!(ui.terminal_size, MAX_TERMINAL_SIZE);
        assert!(!ui.step_terminal_size(true));
        assert!(ui.set_terminal_size(f32::NAN));
        assert_eq!(ui.terminal_size, TERMINAL_SIZE);
        // kept, and read back; a file from before they existed has the usual ones
        let chosen = Ui { code_size: 15, code_colors: CodeColors::Quiet, terminal_size: 14.5, ..Ui::default() };
        assert_eq!(Ui::read(&chosen.write()), chosen);
        let old = Ui::read(r#"{"layout":"side"}"#);
        assert_eq!((old.code_size, old.terminal_size, old.code_colors), (12, 12.5, CodeColors::Ide));
        // a file changed by hand: sizes come back inside, and colors nobody knows are the usual ones
        let odd = Ui::read(r#"{"codeSize":300,"terminalSize":2,"codeColors":"neon"}"#);
        assert_eq!((odd.code_size, odd.terminal_size, odd.code_colors), (24, 8.5, CodeColors::Ide));
        assert_eq!(Ui::read(r#"{"codeSize":-4}"#).code_size, 12, "a size that is no size at all");
    }
}
