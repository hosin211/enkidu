//! The switcher (DESIGN.md, "Layout": "The switcher"): ⌃⇥ held shows every workspace as a card, the most lately
//! used first. Here is what it needs that is not drawing: the cards' order, where the blue edge stands and how
//! ⌃⇥ moves it, where Claude stands in words, and the end of a terminal as a few lines.

use crate::work::pr::{PrLamp, TabPr};
use crate::work::when::{clock_of, since_words};

/// A workspace came on screen: it is the most lately used one now.
pub fn used(recent: &mut Vec<String>, id: &str) {
    if recent.first().map(String::as_str) == Some(id) {
        return;
    }
    recent.retain(|other| other != id);
    recent.insert(0, id.to_string());
}

/// The cards' order: the most lately used first, then the others in the rail's order. `rows` are the workspaces
/// that have a tab (the ones you put away are not among them); what is no longer there is left out.
pub fn order(recent: &[String], rows: &[String]) -> Vec<String> {
    let mut order: Vec<String> = recent.iter().filter(|id| rows.contains(id)).cloned().collect();
    order.extend(rows.iter().filter(|id| !recent.contains(id)).cloned());
    order
}

/// Where the blue edge stands as the switcher opens: on the workspace used before this one (⌃⇥), or on the one
/// used longest ago (⌃⇧⇥). The one on screen is the first card.
pub fn opens_at(count: usize, forward: bool) -> usize {
    match (count, forward) {
        (0 | 1, _) => 0,
        (_, true) => 1,
        (count, false) => count - 1,
    }
}

/// One more ⌃⇥, or ⌃⇧⇥: the next card or the one before, around the ends.
pub fn step(at: usize, count: usize, forward: bool) -> usize {
    match (count, forward) {
        (0, _) => 0,
        (count, true) => (at + 1) % count,
        (count, false) => (at + count - 1) % count,
    }
}

/// A length of time in a word: "12 s", "4 min", "3 h", "2 d".
fn span(ms: i64) -> String {
    let seconds = (ms / 1000).max(0);
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        3600..86_400 => format!("{} h", seconds / 3600),
        _ => format!("{} d", seconds / 86_400),
    }
}

/// Where Claude stands in a workspace, in words: "Needs you", "Working for 12 s", "Done 4 min ago", "Idle since
/// 10:42". `since` is when it came to stand so, when that was seen (a state it had as the app started has no time,
/// and is said without one).
pub fn says(status: &str, since: Option<i64>, now: i64) -> String {
    let (word, time) = match status {
        "needs_you" => ("Needs you", since.map(|at| format!("for {}", span(now - at)))),
        "working" => ("Working", since.map(|at| format!("for {}", span(now - at)))),
        "done" => ("Done", since.map(|at| format!("{} ago", span(now - at)))),
        _ => (
            "Idle",
            since.map(|at| match since_words(at, now).as_str() {
                "since today" => format!("since {}", clock_of(at)),
                other => other.to_string(),
            }),
        ),
    };
    match time {
        Some(time) => format!("{word} {time}"),
        None => word.to_string(),
    }
}

/// The frame of a box drawn with characters (Claude's prompt stands in one): its lines and corners.
fn is_frame(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

/// The end of a terminal as a card shows it: its last `keep` lines that say something. The rows of a drawn box's
/// frame are left out and its sides are taken off what stands between them, the blank rows at the end are dropped,
/// and a run of blank rows inside counts as one.
pub fn tail(rows: &[String], keep: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for row in rows {
        let framed = row.chars().any(is_frame);
        let inside = row.trim_matches(|c: char| is_frame(c) || c == ' ');
        if framed && inside.chars().all(|c| is_frame(c) || c == ' ') {
            // a box's top, bottom or a rule: nothing to read
            continue;
        }
        let line = if framed { inside.to_string() } else { row.trim_end().to_string() };
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    let from = lines.len().saturating_sub(keep);
    let mut kept = lines.split_off(from);
    // (a cut that begins on a blank line begins on the next one)
    if kept.first().is_some_and(|first| first.is_empty()) {
        kept.remove(0);
    }
    kept
}

/// How many files are changed, in words: "1 file", "18 files". Nothing when none is.
pub fn files_words(files: usize) -> Option<String> {
    match files {
        0 => None,
        1 => Some("1 file".to_string()),
        files => Some(format!("{files} files")),
    }
}

/// A workspace's pull request as a card says it: what waits on you there ("#2395: checks failing, 2 remarks
/// waiting"), else its number and how it stands ("#2326 merged").
pub fn pr_words(pr: &TabPr) -> String {
    if pr.waiting {
        return pr.title.trim_start_matches("PR ").to_string();
    }
    let stands = match pr.lamp {
        PrLamp::Red => "checks failing",
        PrLamp::Amber => "checks running",
        PrLamp::Green => "open",
        PrLamp::Violet => "merged",
        PrLamp::Gray => "closed",
    };
    format!("#{} {stands}", pr.number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn the_cards_stand_by_last_use_then_in_the_rails_order() {
        let mut recent = Vec::new();
        for id in ["a", "c", "a", "b"] {
            used(&mut recent, id);
        }
        assert_eq!(recent, ids(&["b", "a", "c"]));
        // on screen again: nothing moves
        used(&mut recent, "b");
        assert_eq!(recent, ids(&["b", "a", "c"]));
        // the ones never used come after, in the rail's order; one that was put away or removed is not there
        assert_eq!(order(&recent, &ids(&["a", "b", "d", "e"])), ids(&["b", "a", "d", "e"]));
        assert_eq!(order(&[], &ids(&["a", "b"])), ids(&["a", "b"]));
    }

    #[test]
    fn it_opens_on_the_one_used_before_and_each_press_goes_on_around_the_ends() {
        // ⌃⇥: the one before this one; ⌃⇧⇥: the one used longest ago
        assert_eq!((opens_at(4, true), opens_at(4, false)), (1, 3));
        // alone, or with nothing: on itself
        assert_eq!((opens_at(1, true), opens_at(1, false), opens_at(0, true)), (0, 0, 0));
        // on: 1, 2, 3, and around to the one on screen
        assert_eq!([1, 2, 3].map(|at| step(at, 4, true)), [2, 3, 0]);
        // back: 1, 0, and around to the last
        assert_eq!([1, 0].map(|at| step(at, 4, false)), [0, 3]);
        assert_eq!(step(0, 0, true), 0);
    }

    #[test]
    fn it_says_where_claude_stands_with_how_long_when_that_was_seen() {
        let now = 1_763_400_000_000;
        let (s, min, h) = (1000, 60_000, 3_600_000);
        assert_eq!(says("needs_you", None, now), "Needs you");
        assert_eq!(says("needs_you", Some(now - 3 * min), now), "Needs you for 3 min");
        assert_eq!(says("working", Some(now - 12 * s), now), "Working for 12 s");
        assert_eq!(says("working", None, now), "Working");
        assert_eq!(says("done", Some(now - 4 * min), now), "Done 4 min ago");
        assert_eq!(says("done", Some(now - 90 * min), now), "Done 1 h ago");
        assert_eq!(says("done", Some(now - 50 * h), now), "Done 2 d ago");
        assert_eq!(says("idle", None, now), "Idle");
        assert_eq!(says("anything else", None, now), "Idle");
        // idle since a moment of today: its clock time; of another day: the day
        let today = says("idle", Some(now - 1000), now);
        let clock = today.strip_prefix("Idle since ").expect("since");
        assert!(clock == "yesterday" || (clock.len() == 5 && clock.as_bytes()[2] == b':'), "{today}");
        assert!(says("idle", Some(now - 72 * h), now).starts_with("Idle since "));
    }

    #[test]
    fn the_end_of_a_terminal_is_its_last_lines_that_say_something() {
        let rows: Vec<String> = [
            "● Update(src/orders/stock.py)",
            "",
            "",
            "● A returned order is counted once now.",
            "",
            "✶ Running the tests… (12s · esc to interrupt)",
            "",
            "╭──────────────────────────────╮",
            "│ > fix the next one           │",
            "╰──────────────────────────────╯",
            "  ? for shortcuts",
            "",
            "",
        ]
        .map(String::from)
        .to_vec();
        // the box's frame is gone, what stood in it stays; the blank rows at the end are dropped
        assert_eq!(
            tail(&rows, 5),
            ["✶ Running the tests… (12s · esc to interrupt)", "", "> fix the next one", "  ? for shortcuts"]
        );
        // fewer rows than asked for: all of them, runs of blanks as one
        assert_eq!(
            tail(&rows[..4], 9),
            ["● Update(src/orders/stock.py)", "", "● A returned order is counted once now."]
        );
        // a rule alone is nothing to read
        assert_eq!(tail(&["────────".to_string(), "> ".to_string(), "────────".to_string()], 4), [">"]);
        assert!(tail(&[], 4).is_empty() && tail(&["".to_string()], 4).is_empty());
    }

    #[test]
    fn it_says_the_changes_and_the_pull_request_in_few_words() {
        assert_eq!(files_words(0), None);
        assert_eq!((files_words(1).as_deref(), files_words(18).as_deref()), (Some("1 file"), Some("18 files")));
        let pr = |lamp, waiting, title: &str| TabPr { number: 2395, lamp, title: title.to_string(), waiting };
        assert_eq!(
            pr_words(&pr(PrLamp::Red, true, "PR #2395: checks failing, 2 remarks waiting")),
            "#2395: checks failing, 2 remarks waiting"
        );
        // nothing waits: its number and how it stands, not its title
        assert_eq!(
            pr_words(&pr(PrLamp::Violet, false, "PR #2395: Export orders to the accounting CSV")),
            "#2395 merged"
        );
        assert_eq!(pr_words(&pr(PrLamp::Green, false, "PR #2395: x")), "#2395 open");
        assert_eq!(pr_words(&pr(PrLamp::Amber, false, "PR #2395: x")), "#2395 checks running");
        assert_eq!(pr_words(&pr(PrLamp::Gray, false, "PR #2395: x")), "#2395 closed");
    }
}
