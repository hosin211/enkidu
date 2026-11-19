//! What is going on in a PR, as its tab in the card says it (DESIGN.md, "The card in front of the board"): who opened
//! it and how big it is, what it says, every check, each reviewer's latest review and who is asked, every review thread
//! (the open ones first, the resolved ones behind a click), the conversation and the latest commits. All from the same
//! read as the board.

use enkidu_core::work::model::{Pr, ThreadLine};
use enkidu_core::work::parse::parse_time;

use super::board::Tone;
use super::markdown::blocks_of;
use super::rich::Block;
use super::sources::ago;

/// A PR's description longer than this many characters shows its start.
pub const LONG_BODY: usize = 900;

/// How a check stands, in gh's buckets, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Fail,
    Cancel,
    Pending,
    Pass,
    Skipping,
}

impl Bucket {
    pub fn of(bucket: &str) -> Bucket {
        match bucket {
            "fail" => Bucket::Fail,
            "cancel" => Bucket::Cancel,
            "pending" => Bucket::Pending,
            "skipping" => Bucket::Skipping,
            _ => Bucket::Pass,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Bucket::Fail => "Failed",
            Bucket::Cancel => "Cancelled",
            Bucket::Pending => "Running",
            Bucket::Pass => "Passed",
            Bucket::Skipping => "Skipped",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Bucket::Fail | Bucket::Cancel => Tone::Red,
            Bucket::Pending => Tone::Amber,
            Bucket::Pass => Tone::Green,
            Bucket::Skipping => Tone::Muted,
        }
    }

    /// It passed or was skipped: behind a click while others didn't.
    pub fn quiet(self) -> bool {
        matches!(self, Bucket::Pass | Bucket::Skipping)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRow {
    pub name: String,
    pub bucket: Bucket,
    /// Its page, to open in the browser.
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRow {
    pub who: String,
    pub bot: bool,
    /// "Approved", "Asked for changes", "Commented", "Dismissed".
    pub word: &'static str,
    pub tone: Tone,
    /// "40 min ago".
    pub when: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRow {
    pub resolved: bool,
    /// "orders/export.py:48".
    pub place: Option<String>,
    pub outdated: bool,
    pub url: Option<String>,
    /// Who started it ("Someone" when GitHub doesn't say), and what they said.
    pub who: String,
    pub text: String,
    /// "1 reply, the latest from sam-lee 2 h ago".
    pub replies: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentRow {
    pub who: Option<String>,
    pub bot: bool,
    pub when: String,
    pub url: String,
    pub text: String,
}

/// A person as their initials on a hue of their own (the same person, the same color, everywhere).
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub initials: String,
    pub hue: f32,
}

impl Person {
    pub fn of(name: &str) -> Person {
        let spaced: String = name.chars().map(|c| if matches!(c, '-' | '_' | '.') { ' ' } else { c }).collect();
        let words: Vec<&str> = spaced.split_whitespace().collect();
        let initials: String = match words.as_slice() {
            [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
            [only] => only.chars().take(2).collect(),
            [] => "?".to_string(),
        };
        // the old hash: each letter's code, times 31, around the color wheel
        let hue = name.to_lowercase().chars().fold(17u32, |hue, c| {
            let mut units = [0u16; 2];
            (hue * 31 + u32::from(c.encode_utf16(&mut units)[0])) % 360
        });
        Person { initials: initials.to_uppercase(), hue: hue as f32 }
    }
}

/// A number as English writes it: 1,234.
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrTab {
    pub number: i64,
    pub url: String,
    /// Who opened it, and when: ("sam-lee", "2 days ago").
    pub opened: Option<(String, String)>,
    /// Its size: ("+412", "−38", "in 9 files").
    pub size: Option<(String, String, String)>,
    /// "5 commits".
    pub commits: String,
    pub labels: Vec<String>,
    /// What the PR says, read from GitHub's Markdown; none when it says nothing.
    pub body: Option<Vec<Block>>,
    /// The description is long: its start is shown, the rest behind a click.
    pub long_body: bool,
    /// How many checks stand how, in the order they are listed: (Fail, 1), (Pass, 4), (Skipping, 1).
    pub check_counts: Vec<(Bucket, usize)>,
    /// The checks that failed, were cancelled or run; then the ones that passed or were skipped.
    pub checks: Vec<CheckRow>,
    pub passed: Vec<CheckRow>,
    pub reviews: Vec<ReviewRow>,
    /// Asked to review, and hasn't yet.
    pub requested: Vec<String>,
    pub open_threads: Vec<ThreadRow>,
    pub resolved_threads: Vec<ThreadRow>,
    pub comments: Vec<CommentRow>,
    /// The latest commits, the newest first: (its first letters, its first line).
    pub recent_commits: Vec<(String, String)>,
    /// "5", or "5 of 12" when there are more than the read brought.
    pub commits_shown: String,
}

impl PrTab {
    pub fn of(pr: &Pr, now: i64) -> PrTab {
        let d = &pr.details;
        let time = |at: &Option<String>| at.as_deref().and_then(parse_time);
        let mut all: Vec<CheckRow> = (d.checks.iter())
            .map(|check| CheckRow {
                name: check.name.clone(),
                bucket: Bucket::of(&check.bucket),
                url: check.url.clone(),
            })
            .collect();
        // by how they stand, then by name as a reader sorts names (capitals among the small letters)
        all.sort_by(|a, b| {
            (a.bucket.cmp(&b.bucket))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        let check_counts = [Bucket::Fail, Bucket::Cancel, Bucket::Pending, Bucket::Pass, Bucket::Skipping]
            .into_iter()
            .map(|bucket| (bucket, all.iter().filter(|check| check.bucket == bucket).count()))
            .filter(|(_, n)| *n > 0)
            .collect();
        let (passed, checks): (Vec<CheckRow>, Vec<CheckRow>) = all.into_iter().partition(|check| check.bucket.quiet());
        let thread = |t: &ThreadLine| ThreadRow {
            resolved: t.resolved,
            place: t.path.as_ref().filter(|path| !path.is_empty()).map(|path| match t.line.filter(|line| *line != 0) {
                Some(line) => format!("{path}:{line}"),
                None => path.clone(),
            }),
            outdated: t.outdated,
            url: t.url.clone().filter(|url| !url.is_empty()),
            who: t.who.clone().unwrap_or_else(|| "Someone".into()),
            text: t.text.clone(),
            replies: (t.replies > 0).then(|| {
                let latest =
                    t.last_who.as_ref().map(|who| format!(", the latest from {who} {}", ago(time(&t.last_at), now)));
                format!("{}{}", count(t.replies, "reply", "replies"), latest.unwrap_or_default())
            }),
        };
        let shown = d.recent_commits.len();
        PrTab {
            number: pr.number,
            url: pr.url.clone(),
            opened: d.author.clone().map(|author| (author, ago(time(&d.created_at), now))),
            size: d.additions.map(|added| {
                let files = d.changed_files.unwrap_or(0);
                (
                    format!("+{}", thousands(added)),
                    format!("−{}", thousands(d.deletions.unwrap_or(0))),
                    format!("in {} {}", files, if files == 1 { "file" } else { "files" }),
                )
            }),
            commits: count(d.commit_count, "commit", "commits"),
            labels: d.labels.clone(),
            body: Some(blocks_of(&d.body)).filter(|blocks| !blocks.is_empty()),
            long_body: d.body.chars().count() > LONG_BODY,
            check_counts,
            checks,
            passed,
            reviews: (d.reviews.iter())
                .map(|review| {
                    let (word, tone) = match review.state.as_str() {
                        "APPROVED" => ("Approved", Tone::Green),
                        "CHANGES_REQUESTED" => ("Asked for changes", Tone::Red),
                        "DISMISSED" => ("Dismissed", Tone::Muted),
                        _ => ("Commented", Tone::Muted),
                    };
                    ReviewRow { who: review.who.clone(), bot: review.bot, word, tone, when: ago(time(&review.at), now) }
                })
                .collect(),
            requested: d.requested.clone(),
            open_threads: d.threads.iter().filter(|t| !t.resolved).map(thread).collect(),
            resolved_threads: d.threads.iter().filter(|t| t.resolved).map(thread).collect(),
            comments: (d.comments.iter())
                .map(|comment| CommentRow {
                    who: comment.who.clone(),
                    bot: comment.bot,
                    when: ago(parse_time(&comment.at), now),
                    url: comment.url.clone(),
                    text: comment.text.clone(),
                })
                .collect(),
            recent_commits: d.recent_commits.iter().map(|c| (c.oid.clone(), c.headline.clone())).collect(),
            commits_shown: match d.commit_count > shown {
                true => format!("{shown} of {}", d.commit_count),
                false => d.commit_count.to_string(),
            },
        }
    }

    /// Its size in one line: "+412 −38 in 9 files".
    pub fn size_words(&self) -> Option<String> {
        self.size.as_ref().map(|(added, removed, files)| format!("{added} {removed} {files}"))
    }

    /// How the checks stand, counted: "1 failed, 4 passed, 1 skipped".
    pub fn check_words(&self) -> String {
        let said: Vec<String> =
            self.check_counts.iter().map(|(bucket, n)| format!("{n} {}", bucket.word().to_lowercase())).collect();
        said.join(", ")
    }

    /// "2 open, 1 resolved".
    pub fn thread_words(&self) -> String {
        match self.resolved_threads.len() {
            0 => format!("{} open", self.open_threads.len()),
            done => format!("{} open, {done} resolved", self.open_threads.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::tests::NOW;
    use crate::work::rich::plain;
    use crate::work::sample;

    fn tab(number: i64) -> PrTab {
        let work = sample();
        let pr = work.prs.iter().find(|pr| pr.number == number).unwrap_or_else(|| panic!("no PR {number}"));
        PrTab::of(pr, NOW)
    }

    /// Shows what is going on in each open PR in its own tab.
    #[test]
    fn says_what_is_going_on_in_a_pr() {
        let pr = tab(2414);
        assert_eq!(pr.opened, Some(("sam-lee".to_string(), "2 days ago".to_string())));
        assert_eq!(pr.size_words().as_deref(), Some("+412 −38 in 9 files"));
        assert_eq!((pr.commits.as_str(), pr.labels.clone()), ("5 commits", vec!["accounting".to_string()]));
        // what it says, from GitHub's Markdown: the template's comment is gone
        let body = plain(pr.body.as_deref().expect("its description"));
        assert!(body.contains("What changes") && body.contains("10,000 lines in under 5 seconds"), "{body}");
        assert!(!body.contains("reminders") && !body.contains("<!--"), "{body}");
        // the failing check first; the ones that passed or were skipped behind a click, by name
        assert_eq!(pr.check_words(), "1 failed, 4 passed, 1 skipped");
        assert_eq!(pr.checks.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["tests (3.12)"]);
        assert_eq!(pr.checks[0].url.as_deref(), Some("https://ci.example.com/run/88123"));
        assert_eq!(
            pr.passed.iter().map(|c| (c.name.as_str(), c.bucket.word())).collect::<Vec<_>>(),
            [
                ("build", "Passed"),
                ("lint", "Passed"),
                ("SonarCloud", "Passed"),
                ("tests (3.11)", "Passed"),
                ("deploy preview", "Skipped")
            ]
        );
        // each reviewer's latest review, and who is asked
        let reviews: Vec<String> = pr
            .reviews
            .iter()
            .map(|r| format!("{} {} {}{}", r.who, r.word, r.when, if r.bot { " bot" } else { "" }))
            .collect();
        assert_eq!(reviews, ["mira-k Commented 40 min ago", "coderabbitai Commented 2 days ago bot"]);
        assert_eq!(pr.requested, ["tom-b"]);
        // the open threads, the resolved one apart
        assert_eq!(pr.thread_words(), "2 open, 1 resolved");
        let first = &pr.open_threads[0];
        assert_eq!(
            (first.place.as_deref(), first.who.as_str(), first.replies.clone()),
            (Some("orders/export.py:48"), "mira-k", None)
        );
        assert_eq!(pr.open_threads[1].replies.as_deref(), Some("1 reply, the latest from sam-lee 2 h ago"));
        assert_eq!(pr.resolved_threads[0].replies.as_deref(), Some("2 replies, the latest from sam-lee yesterday"));
        assert!(pr.resolved_threads[0].resolved && !first.resolved);
        // the conversation and the latest commits
        assert_eq!(pr.comments.len(), 2);
        assert_eq!((pr.comments[1].who.as_deref(), pr.comments[1].when.as_str()), (Some("jonas-qa"), "14 min ago"));
        assert_eq!(pr.recent_commits[0], ("2414abc".to_string(), "Skip rows with a zero total".to_string()));
        assert_eq!(pr.commits_shown, "5");
    }

    /// What the words say in the cases the sample doesn't have.
    #[test]
    fn counts_and_names_as_a_person_would() {
        let mut pr = sample().prs.into_iter().find(|pr| pr.number == 2414).expect("the sample's PR");
        pr.details.additions = Some(12_345);
        pr.details.deletions = None;
        pr.details.changed_files = Some(1);
        pr.details.commit_count = 12;
        pr.details.body = "  \n<!-- only the template -->\n".into();
        pr.details.checks.iter_mut().for_each(|check| check.bucket = "pending".into());
        pr.details.reviews[0].state = "CHANGES_REQUESTED".into();
        pr.details.reviews[1].state = "APPROVED".into();
        pr.details.threads.retain(|t| !t.resolved);
        pr.details.threads[0].path = None;
        pr.details.threads[0].who = None;
        pr.details.threads[1].replies = 3;
        pr.details.threads[1].last_who = None;
        let tab = PrTab::of(&pr, NOW);
        assert_eq!(tab.size_words().as_deref(), Some("+12,345 −0 in 1 file"));
        assert_eq!((tab.commits.as_str(), tab.commits_shown.as_str()), ("12 commits", "5 of 12"));
        assert_eq!(tab.body, None, "a description that is only the template says nothing");
        assert_eq!(tab.check_words(), "6 running");
        assert!(tab.passed.is_empty() && tab.checks.len() == 6);
        assert_eq!((tab.reviews[0].word, tab.reviews[0].tone), ("Asked for changes", Tone::Red));
        assert_eq!((tab.reviews[1].word, tab.reviews[1].tone), ("Approved", Tone::Green));
        assert_eq!(tab.thread_words(), "2 open");
        assert_eq!((tab.open_threads[0].place.clone(), tab.open_threads[0].who.as_str()), (None, "Someone"));
        assert_eq!(tab.open_threads[1].replies.as_deref(), Some("3 replies"));
        // a long description is told from a short one by its letters
        pr.details.body = "word ".repeat(181);
        assert!(PrTab::of(&pr, NOW).long_body);
        pr.details.body = "word ".repeat(180);
        assert!(!PrTab::of(&pr, NOW).long_body);
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// A person's badge: their initials, and a hue that is theirs everywhere.
    #[test]
    fn a_person_is_their_initials_on_a_hue_of_their_own() {
        assert_eq!(Person::of("mira-k").initials, "MK");
        assert_eq!(Person::of("Sam Lee").initials, "SL");
        assert_eq!(Person::of("coderabbitai").initials, "CO");
        assert_eq!(Person::of("jonas.weber_qa").initials, "JW");
        assert_eq!(Person::of("").initials, "?");
        // the old hash, by hand for "ab": ((17*31 + 97) % 360 * 31 + 98) % 360
        let by_hand = ((17 * 31 + 97) % 360 * 31 + 98) % 360;
        assert_eq!(Person::of("ab").hue, by_hand as f32);
        assert_eq!(Person::of("Mira-K").hue, Person::of("mira-k").hue, "whatever the capitals");
        assert_ne!(Person::of("mira-k").hue, Person::of("sam-lee").hue);
    }
}
