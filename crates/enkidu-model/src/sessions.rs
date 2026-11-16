//! Which Claudes come back at launch (item 2.14).

/// The workspaces whose Claude starts at launch, in order: the one on screen first, the others as they are listed.
/// `kept` is the list written when the app last ran, the workspaces that had a Claude then; None the first time, when
/// every workspace gets its Claude. A workspace that is gone is left out; one that had no Claude gets it when you
/// open it.
pub fn to_bring_back(workspaces: &[String], kept: Option<&[String]>, on_screen: Option<&str>) -> Vec<String> {
    let mut back: Vec<String> =
        workspaces.iter().filter(|id| kept.is_none_or(|kept| kept.contains(id))).cloned().collect();
    // a stable sort: the one on screen moves to the front, the rest keep their order
    back.sort_by_key(|id| Some(id.as_str()) != on_screen);
    back
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn the_first_time_every_workspace_comes_back_the_one_on_screen_first() {
        assert_eq!(to_bring_back(&ids(&["w1", "w2", "w3"]), None, Some("w2")), ids(&["w2", "w1", "w3"]));
        assert_eq!(to_bring_back(&ids(&["w1", "w2"]), None, None), ids(&["w1", "w2"]));
    }

    #[test]
    fn afterwards_only_the_ones_that_had_a_claude_and_none_that_is_gone() {
        let kept = ids(&["w3", "w9-removed", "w1"]);
        assert_eq!(to_bring_back(&ids(&["w1", "w2", "w3"]), Some(&kept), None), ids(&["w1", "w3"]));
        // the one on screen had none: it is not started here (it gets its Claude as it opens)
        assert_eq!(to_bring_back(&ids(&["w1", "w2", "w3"]), Some(&kept), Some("w2")), ids(&["w1", "w3"]));
        assert_eq!(to_bring_back(&ids(&["w1", "w2", "w3"]), Some(&kept), Some("w3")), ids(&["w3", "w1"]));
    }

    #[test]
    fn an_empty_list_brings_nothing_back() {
        assert!(to_bring_back(&ids(&["w1"]), Some(&[]), Some("w1")).is_empty());
    }
}
