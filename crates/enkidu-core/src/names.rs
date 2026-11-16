//! The order names and paths are shown in: names as a reader orders them, and paths as a tree of their folders lists
//! them. The changed files come from git in the second order (`git::changes`), so a list of them, the tree made of
//! them and the diffs stacked from them all run the same way.

use std::cmp::Ordering;

/// A letter without its accent and in lower case, as names are compared: "É" and "e" stand together.
fn folded(c: char) -> char {
    let plain = match c {
        'À'..='Å' | 'à'..='å' => 'a',
        'Ç' | 'ç' => 'c',
        'È'..='Ë' | 'è'..='ë' => 'e',
        'Ì'..='Ï' | 'ì'..='ï' => 'i',
        'Ñ' | 'ñ' => 'n',
        'Ò'..='Ö' | 'ò'..='ö' | 'Ø' | 'ø' => 'o',
        'Ù'..='Ü' | 'ù'..='ü' => 'u',
        'Ý' | 'ý' | 'ÿ' => 'y',
        other => other,
    };
    plain.to_lowercase().next().unwrap_or(plain)
}

/// Names as a reader orders them: capitals beside their small letters, and numbers by their value ("file9" before
/// "file10"). Names that only differ in case or accents keep one fixed order.
pub fn by_name(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                // a run of digits is a number: without its leading zeros the longer one is the greater
                let number = |digits: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut run = String::new();
                    while let Some(digit) = digits.peek().copied().filter(char::is_ascii_digit) {
                        run.push(digit);
                        digits.next();
                    }
                    run.trim_start_matches('0').to_string()
                };
                let (x, y) = (number(&mut left), number(&mut right));
                match x.len().cmp(&y.len()).then_with(|| x.cmp(&y)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                match folded(x).cmp(&folded(y)) {
                    Ordering::Equal => {}
                    other => return other,
                }
                left.next();
                right.next();
            }
        }
    }
}

/// Paths in the order a tree of their folders lists them: in every folder its folders first, then its files, both by
/// name. "src/orders/stock.py" stands before "src/app.py", and both before "notes.md".
pub fn in_tree_order(a: &str, b: &str) -> Ordering {
    // the folders both paths start with need no look: neighbours in a list share most of their path, and a list of
    // thousands is put in order (or found to be in it) by comparing neighbours
    let shared = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let from = a.as_bytes()[..shared].iter().rposition(|byte| *byte == b'/').map_or(0, |at| at + 1);
    let (a, b) = (&a[from..], &b[from..]);
    let (mut left, mut right) = (a.split('/'), b.split('/'));
    let (mut x, mut y) = (left.next(), right.next());
    loop {
        match (x, y) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(name_a), Some(name_b)) => {
                let (next_a, next_b) = (left.next(), right.next());
                // a step with more of the path after it is a folder, and a folder stands before a file
                match (next_a.is_some(), next_b.is_some()) {
                    (true, false) => return Ordering::Less,
                    (false, true) => return Ordering::Greater,
                    _ => {}
                }
                match by_name(name_a, name_b) {
                    Ordering::Equal => {}
                    other => return other,
                }
                (x, y) = (next_a, next_b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stand_as_a_reader_orders_them() {
        assert_eq!(by_name("File2", "file10"), Ordering::Less);
        assert_eq!(by_name("Émile", "evan"), Ordering::Less, "an accent doesn't send a name past z");
        assert_eq!(by_name("a01", "a1"), by_name("a01", "a1"), "equal numbers keep one fixed order");
        assert_ne!(by_name("a01", "a1"), Ordering::Equal);
        assert_eq!(by_name("readme", "README"), Ordering::Greater, "the same name in two cases: capitals first");
    }

    #[test]
    fn paths_stand_as_a_tree_of_their_folders_lists_them() {
        let mut paths = vec![
            "zebra.txt",
            "notes.md",
            "src/app.py",
            "src/orders/tests/test_stock.py",
            "src/orders/stock.py",
            "src/Orders2/a.py",
            "src/orders10/a.py",
            "docs/guide.md",
            "a-b/x.py",
            "a/x.py",
            "a.py",
            "File10.txt",
            "file9.txt",
        ];
        paths.sort_by(|a, b| in_tree_order(a, b));
        assert_eq!(
            paths,
            [
                // the folders of the top first, by name
                "a/x.py",
                "a-b/x.py",
                "docs/guide.md",
                // in a folder: its folders first (numbers by their value, capitals beside small letters)
                "src/orders/tests/test_stock.py",
                "src/orders/stock.py",
                "src/Orders2/a.py",
                "src/orders10/a.py",
                "src/app.py",
                // then the top's own files
                "a.py",
                "file9.txt",
                "File10.txt",
                "notes.md",
                "zebra.txt",
            ]
        );
        // the same path is the same place; a folder stands before a file of its own name (a file deleted, and a
        // folder made where it was)
        assert_eq!(in_tree_order("src/app.py", "src/app.py"), Ordering::Equal);
        assert_eq!(in_tree_order("a/b.py", "a"), Ordering::Less);
        assert_eq!(in_tree_order("a", "a/b.py"), Ordering::Greater);
    }
}
