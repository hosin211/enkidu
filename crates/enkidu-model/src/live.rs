//! Live as the window shows it (item 4.5): the button's five states with their words and tooltips, which way a
//! project's Live is switching (a second press meanwhile does nothing), and what a toast says once it switched. One
//! workspace of a project is Live at a time: its changes are copied into the project's main folder, where the dev
//! environment runs, and follow every edit.

use std::collections::HashMap;

/// What the Live button of a workspace shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveState {
    /// "Go live".
    Off,
    /// This workspace's changes are being copied in.
    Starting,
    /// It is Live: pointing at the button turns it into Stop.
    Live,
    /// The main folder is being put back.
    Stopping,
    /// Live is switching in another workspace of the project: nothing to press here meanwhile.
    Waiting,
}

impl LiveState {
    /// The state of `workspace`'s button. `live` is its project's Live workspace; `switching` is where the project's
    /// Live is going while it switches (to no workspace: it is stopping).
    pub fn of(workspace: &str, live: Option<&str>, switching: Option<&Option<String>>) -> LiveState {
        let is_live = live == Some(workspace);
        match switching {
            Some(target) if target.as_deref() == Some(workspace) => LiveState::Starting,
            Some(_) if is_live => LiveState::Stopping,
            Some(_) => LiveState::Waiting,
            None if is_live => LiveState::Live,
            None => LiveState::Off,
        }
    }

    /// The state in a word, for tests and the accessibility tree.
    pub fn name(self) -> &'static str {
        match self {
            LiveState::Off => "off",
            LiveState::Starting => "starting",
            LiveState::Live => "live",
            LiveState::Stopping => "stopping",
            LiveState::Waiting => "waiting",
        }
    }

    /// Whether a press does something: only when nothing is switching.
    pub fn enabled(self) -> bool {
        matches!(self, LiveState::Off | LiveState::Live)
    }

    /// The words on the button. `pointed`: the pointer is on it, which turns Live into Stop.
    pub fn words(self, pointed: bool) -> &'static str {
        match self {
            LiveState::Off | LiveState::Waiting => "Go live",
            LiveState::Starting => "Starting…",
            LiveState::Stopping => "Stopping…",
            LiveState::Live if pointed => "Stop",
            LiveState::Live => "Live",
        }
    }

    /// The tooltip: what a press would do, or what is happening. `project` is the project's name, `other` the
    /// project's other workspace that is Live now (its changes go back first).
    pub fn tip(self, workspace: &str, project: Option<&str>, other: Option<&str>) -> String {
        let main = project.map_or_else(|| "the main folder".to_string(), |name| format!("{name}'s main folder"));
        match self {
            LiveState::Live => {
                format!("{workspace}'s changes are in {main} and follow every edit. Click to put the main folder back.")
            }
            LiveState::Off => {
                let first = other.map(|other| format!(". {other}'s go back first.")).unwrap_or_default();
                format!(
                    "Copy {workspace}'s changes into {main}, where the dev environment runs, and keep them in step{first}"
                )
            }
            LiveState::Waiting => "Live is switching in another workspace".to_string(),
            LiveState::Starting => format!("Copying {workspace}'s changes into {main}"),
            LiveState::Stopping => "Putting the main folder back".to_string(),
        }
    }
}

/// Where each project's Live is switching to while it switches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Switching(HashMap<String, Option<String>>);

impl Switching {
    /// A press: Live of `project` goes to `target` (none: it stops). False when it is switching already: that
    /// press does nothing.
    pub fn start(&mut self, project: &str, target: Option<&str>) -> bool {
        if self.0.contains_key(project) {
            return false;
        }
        self.0.insert(project.to_string(), target.map(String::from));
        true
    }

    /// The switch ended, whichever way.
    pub fn end(&mut self, project: &str) {
        self.0.remove(project);
    }

    /// Where `project`'s Live is going, while it switches.
    pub fn of(&self, project: &str) -> Option<&Option<String>> {
        self.0.get(project)
    }
}

/// The toast once Live switched: "stock-fix is Live" or "Live stopped", and under it what happened to the main
/// folder, then what the project's own command printed.
pub fn switched(live: Option<&str>, note: &str, output: &str) -> (String, Option<String>) {
    let title = live.map_or_else(|| "Live stopped".to_string(), |name| format!("{name} is Live"));
    let parts: Vec<&str> = [note.trim(), output.trim()].into_iter().filter(|part| !part.is_empty()).collect();
    (title, (!parts.is_empty()).then(|| parts.join("\n\n")))
}

/// The toast's title when Live couldn't switch; the reason goes under it.
pub const DID_NOT_SWITCH: &str = "Live didn't switch";

/// The title of what Live has to say while it follows a workspace: it set an edit of the main folder's own aside, or
/// couldn't copy a change.
pub fn note_title(error: bool) -> &'static str {
    if error { "Live didn't copy a change" } else { "Kept a main-folder edit" }
}

/// The palette's command for the workspace on screen: "Stop Live for stock-fix" while it is Live, else "Go live with
/// stock-fix".
pub fn command(workspace: &str, live: bool) -> String {
    if live { format!("Stop Live for {workspace}") } else { format!("Go live with {workspace}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_says_which_way_live_is_switching() {
        // nothing switching
        assert_eq!(LiveState::of("w1", None, None), LiveState::Off);
        assert_eq!(LiveState::of("w1", Some("w1"), None), LiveState::Live);
        assert_eq!(LiveState::of("w1", Some("w2"), None), LiveState::Off, "another one is Live");
        // going Live here
        let to_w1 = Some("w1".to_string());
        assert_eq!(LiveState::of("w1", None, Some(&to_w1)), LiveState::Starting);
        assert_eq!(
            LiveState::of("w2", Some("w2"), Some(&to_w1)),
            LiveState::Stopping,
            "the one that was Live goes back first"
        );
        assert_eq!(LiveState::of("w3", Some("w2"), Some(&to_w1)), LiveState::Waiting);
        // stopping
        assert_eq!(LiveState::of("w1", Some("w1"), Some(&None)), LiveState::Stopping);
        assert_eq!(LiveState::of("w2", Some("w1"), Some(&None)), LiveState::Waiting);
        // only the two calm states take a press
        let enabled: Vec<bool> =
            [LiveState::Off, LiveState::Starting, LiveState::Live, LiveState::Stopping, LiveState::Waiting]
                .map(LiveState::enabled)
                .to_vec();
        assert_eq!(enabled, [true, false, true, false, false]);
    }

    #[test]
    fn its_words_and_its_tooltip_in_each_state() {
        assert_eq!(LiveState::Off.words(false), "Go live");
        assert_eq!(LiveState::Waiting.words(true), "Go live");
        assert_eq!(LiveState::Starting.words(false), "Starting…");
        assert_eq!(LiveState::Stopping.words(false), "Stopping…");
        assert_eq!(LiveState::Live.words(false), "Live");
        assert_eq!(LiveState::Live.words(true), "Stop", "pointing at it turns it into Stop");
        assert_eq!(LiveState::Off.words(true), "Go live");
        let names = [LiveState::Off, LiveState::Starting, LiveState::Live, LiveState::Stopping, LiveState::Waiting]
            .map(LiveState::name);
        assert_eq!(names, ["off", "starting", "live", "stopping", "waiting"]);

        assert_eq!(
            LiveState::Off.tip("stock-fix", Some("shop"), None),
            "Copy stock-fix's changes into shop's main folder, where the dev environment runs, and keep them in step"
        );
        assert_eq!(
            LiveState::Off.tip("stock-fix", Some("shop"), Some("pr-2175")),
            "Copy stock-fix's changes into shop's main folder, where the dev environment runs, and keep them in step. \
             pr-2175's go back first."
        );
        assert_eq!(
            LiveState::Live.tip("stock-fix", Some("shop"), None),
            "stock-fix's changes are in shop's main folder and follow every edit. Click to put the main folder back."
        );
        assert_eq!(
            LiveState::Starting.tip("stock-fix", None, None),
            "Copying stock-fix's changes into the main folder"
        );
        assert_eq!(LiveState::Stopping.tip("stock-fix", Some("shop"), None), "Putting the main folder back");
        assert_eq!(LiveState::Waiting.tip("stock-fix", Some("shop"), None), "Live is switching in another workspace");
    }

    /// Shows which way Live is switching, and ignores a second click meanwhile.
    #[test]
    fn a_second_press_while_it_switches_does_nothing() {
        let mut switching = Switching::default();
        assert!(switching.start("p1", Some("w1")));
        assert_eq!(switching.of("p1"), Some(&Some("w1".to_string())));
        assert!(!switching.start("p1", None), "a press meanwhile is ignored");
        assert_eq!(switching.of("p1"), Some(&Some("w1".to_string())), "and changes nothing");
        // another project switches on its own
        assert!(switching.start("p2", None));
        assert_eq!(switching.of("p2"), Some(&None));
        switching.end("p1");
        assert_eq!(switching.of("p1"), None);
        assert!(switching.start("p1", None), "once it ended, a press counts again");
        assert_eq!(switching.of("p1"), Some(&None));
    }

    /// Switches Live and says what it did to the main folder, then the command output.
    #[test]
    fn the_toast_says_what_it_did_to_the_main_folder_then_what_the_command_printed() {
        assert_eq!(
            switched(Some("one"), "Copied 2 files with its changes into ~/code/shop.", "$ on\nstarted\n"),
            (
                "one is Live".to_string(),
                Some("Copied 2 files with its changes into ~/code/shop.\n\n$ on\nstarted".to_string())
            )
        );
        assert_eq!(
            switched(None, "  The main folder is back as it was.  ", ""),
            ("Live stopped".to_string(), Some("The main folder is back as it was.".to_string()))
        );
        assert_eq!(switched(None, " ", "\n"), ("Live stopped".to_string(), None), "nothing to add is nothing said");
        assert_eq!(DID_NOT_SWITCH, "Live didn't switch");
        assert_eq!(note_title(false), "Kept a main-folder edit");
        assert_eq!(note_title(true), "Live didn't copy a change");
    }

    #[test]
    fn the_palettes_command_names_the_workspace() {
        assert_eq!(command("stock-fix", false), "Go live with stock-fix");
        assert_eq!(command("stock-fix", true), "Stop Live for stock-fix");
    }
}
