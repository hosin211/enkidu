//! What you wrote about each card, kept until it goes (DESIGN.md, "Tell Claude": "What you write is kept until it goes,
//! also across a restart; then it is cleared"): your words about the task, per item whether it goes with the message
//! and your instruction for it, the Claude you picked, the whole message as you edited it. Kept in `drafts.json` in the
//! app's data folder.

use std::collections::{BTreeMap, HashMap};

use enkidu_model::work::claudes::ClaudeChoice;
use enkidu_model::work::compose::Draft;
use gpui_kit::{App, Context, Entity};

#[derive(Default, Debug, PartialEq)]
pub struct Drafts {
    by_card: BTreeMap<String, Draft>,
}

impl Drafts {
    /// The drafts as they were kept; a file that can't be read is no drafts.
    pub fn read(text: &str) -> Drafts {
        Drafts { by_card: serde_json::from_str(text).unwrap_or_default() }
    }

    pub fn write(&self) -> String {
        serde_json::to_string(&self.by_card).unwrap_or_else(|_| "{}".into())
    }

    /// A card's draft; an empty one for a card you wrote nothing about.
    pub fn of(&self, card: &str) -> Draft {
        self.by_card.get(card).cloned().unwrap_or_default()
    }

    pub fn get(&self, card: &str) -> Option<&Draft> {
        self.by_card.get(card)
    }

    /// Each card's Claude as you picked it: the board's cards show that one.
    pub fn chosen(&self) -> HashMap<String, ClaudeChoice> {
        (self.by_card.iter())
            .filter_map(|(card, draft)| Some((card.clone(), ClaudeChoice::read(draft.claude.as_deref()?))))
            .collect()
    }

    /// Keeps a card's draft; one with nothing in it is forgotten. Says whether anything changed.
    pub fn set(&mut self, card: &str, draft: Draft, cx: &mut Context<Self>) -> bool {
        let empty = draft == Draft::default();
        let changed = match (self.by_card.get(card), empty) {
            (None, true) => false,
            (Some(had), false) => *had != draft,
            _ => true,
        };
        if changed {
            match empty {
                true => self.by_card.remove(card),
                false => self.by_card.insert(card.to_string(), draft),
            };
            cx.notify();
        }
        changed
    }

    /// Changes a card's draft in place.
    pub fn change(drafts: &Entity<Drafts>, card: &str, cx: &mut App, change: impl FnOnce(&mut Draft)) {
        drafts.update(cx, |drafts, cx| {
            let mut draft = drafts.of(card);
            change(&mut draft);
            drafts.set(card, draft, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enkidu_model::work::compose::ItemDraft;
    use gpui_kit::{AppContext as _, TestAppContext};

    /// What you write is kept, also across a restart: the drafts are text and come back as they were.
    #[gpui_kit::test]
    fn a_draft_is_kept_as_text_and_an_empty_one_is_forgotten(cx: &mut TestAppContext) {
        let drafts = cx.new(|_| Drafts::default());
        let changes = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = changes.clone();
        cx.update(|cx| cx.observe(&drafts, move |_, _| seen.set(seen.get() + 1)).detach());
        cx.update(|cx| {
            Drafts::change(&drafts, "ticket:SH-1", cx, |draft| draft.note = "Finish the export first.".into());
        });
        cx.update(|cx| {
            Drafts::change(&drafts, "ticket:SH-1", cx, |draft| {
                draft.items.insert("thread:t2".into(), ItemDraft { include: Some(false), note: None });
                draft.claude = Some("s-w2".into());
            });
        });
        cx.run_until_parked();
        assert_eq!(changes.get(), 2, "each change is told");
        // the same again changes nothing, and nor does a card you wrote nothing about: nobody is told
        cx.update(|cx| {
            Drafts::change(&drafts, "ticket:SH-1", cx, |draft| draft.claude = Some("s-w2".into()));
            Drafts::change(&drafts, "ticket:SH-2", cx, |_| {});
        });
        cx.run_until_parked();
        assert_eq!(changes.get(), 2);
        let text = drafts.read_with(cx, |drafts, _| drafts.write());
        assert_eq!(
            text,
            r#"{"ticket:SH-1":{"note":"Finish the export first.","items":{"thread:t2":{"include":false}},"claude":"s-w2"}}"#
        );
        let back = Drafts::read(&text);
        assert_eq!(back.of("ticket:SH-1").note, "Finish the export first.");
        assert!(!back.of("ticket:SH-1").includes("thread:t2") && back.of("ticket:SH-1").includes("thread:t1"));
        let picked = ClaudeChoice::Session("s-w2".to_string());
        assert_eq!(back.chosen(), HashMap::from([("ticket:SH-1".to_string(), picked)]));
        assert_eq!(back.of("ticket:SH-9"), Draft::default());
        // emptied, it is forgotten
        cx.update(|cx| Drafts::change(&drafts, "ticket:SH-1", cx, |draft| *draft = Draft::default()));
        assert_eq!(drafts.read_with(cx, |drafts, _| drafts.write()), "{}");
        // a file that isn't drafts is no drafts
        assert_eq!(Drafts::read("not json"), Drafts::default());
    }
}
