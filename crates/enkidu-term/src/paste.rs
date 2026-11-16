//! What a paste sends to the program.

const START: &str = "\x1b[200~";
const END: &str = "\x1b[201~";

/// The bytes a paste of `text` sends, in one write. New lines become carriage returns, as a terminal sends Enter.
/// When the program turned on bracketed paste (as Claude Code and shells do), the text is wrapped so the program
/// takes it as one paste, not as typed keys; markers inside the text are dropped, so a paste can't end itself early.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if !bracketed {
        return text.into_bytes();
    }
    let inner = text.replace(START, "").replace(END, "");
    let mut out = String::with_capacity(inner.len() + START.len() + END.len());
    out.push_str(START);
    out.push_str(&inner);
    out.push_str(END);
    out.into_bytes()
}

/// What files dropped from Finder type: their paths, each escaped as a shell reads it (a backslash before anything
/// but letters, digits and `/._-+,:@%`; a path with a control character in single quotes), joined by spaces. Claude
/// Code takes a dropped image's path as the image, as in Terminal.app and iTerm.
pub fn dropped_paths<P: AsRef<std::path::Path>>(paths: &[P]) -> String {
    let one = |path: &P| {
        let path = path.as_ref().to_string_lossy();
        if path.chars().any(char::is_control) {
            return format!("'{}'", path.replace('\'', "'\\''"));
        }
        let mut out = String::with_capacity(path.len() + 8);
        for c in path.chars() {
            if c.is_ascii() && !c.is_ascii_alphanumeric() && !"/._-+,:@%".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    paths.iter().map(one).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracketed_paste_is_one_block_with_its_lines_kept() {
        let pasted = paste_bytes("first line\nsecond line\r\nthird", true);
        assert_eq!(pasted, b"\x1b[200~first line\rsecond line\rthird\x1b[201~");
    }

    #[test]
    fn a_paste_cannot_end_its_own_bracket() {
        let pasted = paste_bytes("a\x1b[201~rm -rf x\n", true);
        assert_eq!(pasted, b"\x1b[200~arm -rf x\r\x1b[201~");
    }

    #[test]
    fn dropped_files_type_their_paths_as_a_shell_reads_them() {
        assert_eq!(dropped_paths(&["/Users/h/Desktop/shot 1.png"]), "/Users/h/Desktop/shot\\ 1.png");
        assert_eq!(dropped_paths(&["/tmp/it's (2) $x & y!.txt"]), r"/tmp/it\'s\ \(2\)\ \$x\ \&\ y\!.txt");
        assert_eq!(
            dropped_paths(&["/Users/h/صورة.png", "/tmp/a,b-c_d+e@f%g:h"]),
            "/Users/h/صورة.png /tmp/a,b-c_d+e@f%g:h"
        );
        assert_eq!(dropped_paths(&["/tmp/odd\nname"]), "'/tmp/odd\nname'");
    }

    #[test]
    fn without_brackets_the_text_goes_as_it_is() {
        assert_eq!(paste_bytes("echo hi\n", false), b"echo hi\r");
        assert_eq!(paste_bytes("ب ت", false), "ب ت".as_bytes());
    }
}
