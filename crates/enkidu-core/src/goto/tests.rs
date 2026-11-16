//! What a language server names, as the app takes it. There are no rules of the app's own to test: a name's place
//! is its language server's to find (`lsp`).

use super::*;

fn method(name: &str, line: u32, class: &str) -> Def {
    Def { name: name.into(), kind: Kind::Method, line, container: Some(class.into()) }
}

/// Phases 12 and 13: what a server names is where the name is defined. What stands on a line (the kind, the class)
/// is said with it where it is known; a library's place is no place here; and while the server can't be asked, the
/// answer says so.
#[test]
fn a_language_servers_answer_is_taken_as_the_names_places() {
    // `item.save()` on line 25 of a view: `save` stands at 13 to 17
    let view = "orders/views/stock_item_view.py";
    let ask = Ask { path: view.into(), line: 25, column: 14 };
    // what stands on the lines the server may name: two methods; nothing is known of the others
    let on_line = |path: &str, line: usize| match (path, line) {
        ("orders/models.py", 7) => Some(method("save", 7, "StockItem")),
        ("billing/models.py", 2) => Some(method("save", 2, "Invoice")),
        _ => None,
    };
    let take = |named: Vec<(Option<String>, usize)>| from_server(&ask, "save", (13, 17), named, &on_line);
    let at = |path: &str, line: usize| (Some(path.to_string()), line);

    // it names nothing, or only the line the question is on: no link, and that is its answer
    for named in [Vec::new(), vec![at(view, 25)]] {
        let found = take(named);
        assert!(found.places.is_empty() && !found.later);
        assert_eq!((found.word.as_str(), found.start, found.end), ("save", 13, 17));
    }

    // a method of the workspace, with what stands on its line
    let save = Place {
        path: "orders/models.py".into(),
        line: 7,
        name: "save".into(),
        kind: Some(Kind::Method),
        container: Some("StockItem".into()),
    };
    assert_eq!(take(vec![at("orders/models.py", 7)]).places, std::slice::from_ref(&save));
    // a line nothing is known of (the line that sets a variable): a place under the name asked about
    let variable = Place { path: view.into(), line: 24, name: "save".into(), kind: None, container: None };
    assert_eq!(take(vec![at(view, 24)]).places, [variable]);

    // only places outside the workspace (a library's file): no link
    let found = take(vec![(None, 812)]);
    assert!(found.places.is_empty() && !found.later);
    // several: each place of the workspace once, the outside one left out, and the question's own line too
    let named = vec![at("orders/models.py", 7), (None, 3), at("orders/models.py", 7), at("billing/models.py", 2)];
    let found = take([named, vec![at(view, 25)]].concat());
    assert_eq!(found.places.len(), 2);
    assert_eq!(found.places[0], save);
    assert_eq!(
        (found.places[1].path.as_str(), found.places[1].container.as_deref()),
        ("billing/models.py", Some("Invoice"))
    );
    // never more than the list can show
    let many: Vec<(Option<String>, usize)> = (1..=MAX_PLACES + 20).map(|line| at("orders/models.py", line)).collect();
    assert_eq!(take(many).places.len(), MAX_PLACES);

    // the server can't be asked yet: no place, and the question is worth asking again
    let waiting = not_yet("save", (13, 17));
    assert!(waiting.places.is_empty() && waiting.later);
    assert_eq!((waiting.word.as_str(), waiting.start, waiting.end), ("save", 13, 17));
}

/// What is asked about: the name around a byte of a line. And which files have a server in the app.
#[test]
fn the_name_asked_about_is_the_one_around_the_pointer() {
    assert_eq!(name_at("    item.save()", 9), Some((9, 13)));
    assert_eq!(name_at("    item.save()", 13), Some((9, 13)), "right after the name");
    assert_eq!(name_at("    item.save()", 4), Some((4, 8)));
    assert_eq!(name_at("x = 12 + y", 5), None, "a number is no name");
    assert_eq!(name_at("a  b", 2), None);
    assert_eq!(name_at("título = 1", 3), Some((0, 7)), "letters of more than one byte");
    assert_eq!(name_at("short", 400), Some((0, 5)), "past the line's end");
    assert!(is_python("orders/models.py") && is_python("stubs/x.PYI"));
    assert!(!is_python("web/a.vue") && !is_python("web/a.js") && !is_python("tools/build.rs") && !is_python("py"));
}
