//! How each source of the Work index stands (item 5.1a). GitHub and Jira are read for every project on their own; the
//! page shows one lamp a source (the worst of its projects), says how old what you see is, and when a source can't be
//! read it says so first: what is wrong, what to do, and how old the data under it is.

use std::collections::{HashMap, HashSet};

pub use enkidu_core::work::model::{Health, Snapshot};

/// A source the Work index reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Github,
    Jira,
}

impl Source {
    pub const BOTH: [Source; 2] = [Source::Github, Source::Jira];

    /// As the core writes it in a `Health`.
    pub fn key(self) -> &'static str {
        match self {
            Source::Github => "github",
            Source::Jira => "jira",
        }
    }

    /// As it is called on the page.
    pub fn name(self) -> &'static str {
        match self {
            Source::Github => "GitHub",
            Source::Jira => "Jira",
        }
    }

    fn of(key: &str) -> Option<Source> {
        Source::BOTH.into_iter().find(|source| source.key() == key)
    }
}

/// How a source's lamp looks (DESIGN.md: "green read, amber reading, red can't read, hollow off").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    /// Read: green.
    Ok,
    /// Can't be read: red.
    Error,
    /// Not read yet.
    Pending,
    /// Not read for any project: hollow.
    Off,
    /// Being read right now: amber.
    Reading,
}

impl Lamp {
    /// The state in a word, for tests and the accessibility tree.
    pub fn name(self) -> &'static str {
        match self {
            Lamp::Ok => "ok",
            Lamp::Error => "error",
            Lamp::Pending => "pending",
            Lamp::Off => "off",
            Lamp::Reading => "reading",
        }
    }
}

/// One source over every project: its worst state, its newest good read, who it reads as, and the projects it
/// can't be read for.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceHealth {
    /// ok, error or pending: the worst of its projects.
    pub state: String,
    pub error: Option<String>,
    pub fix: Option<String>,
    pub last_ok: Option<i64>,
    pub account: Option<String>,
    pub broken: Vec<Health>,
}

/// One source's health over every project that reads it (none: it isn't read for any).
pub fn source_health(health: &[Health], source: Source) -> Option<SourceHealth> {
    let mine: Vec<&Health> = health.iter().filter(|h| h.source == source.key() && h.state != "off").collect();
    let rank = |state: &str| match state {
        "error" => 0,
        "pending" => 1,
        "ok" => 2,
        _ => 3,
    };
    // the first of the worst, as the old stable sort gave it
    let worst = mine.iter().min_by_key(|h| rank(&h.state))?;
    Some(SourceHealth {
        state: worst.state.clone(),
        error: worst.error.clone(),
        fix: worst.fix.clone(),
        last_ok: mine.iter().filter_map(|h| h.last_ok).filter(|at| *at > 0).max(),
        account: worst.account.clone().or_else(|| mine.iter().find_map(|h| h.account.clone())),
        broken: mine.iter().filter(|h| h.state == "error").map(|h| (*h).clone()).collect(),
    })
}

/// The lamp of a source: hollow when it isn't read at all, amber while a read runs, else as it stands.
pub fn lamp(health: Option<&SourceHealth>, checking: bool) -> Lamp {
    match health {
        None => Lamp::Off,
        Some(_) if checking => Lamp::Reading,
        Some(health) => match health.state.as_str() {
            "ok" => Lamp::Ok,
            "error" => Lamp::Error,
            "pending" => Lamp::Pending,
            _ => Lamp::Off,
        },
    }
}

/// "2 min ago", "3 h ago", "yesterday", "5 days ago"; nothing for no time.
pub fn ago(at: Option<i64>, now: i64) -> String {
    let Some(at) = at.filter(|at| *at != 0) else { return String::new() };
    let seconds = (((now - at) as f64) / 1000.).round().max(0.);
    if seconds < 45. {
        return "just now".into();
    }
    let minutes = (seconds / 60.).round();
    if minutes < 60. {
        return format!("{minutes} min ago");
    }
    let hours = (minutes / 60.).round();
    if hours < 24. {
        return format!("{hours} h ago");
    }
    let days = (hours / 24.).round();
    if days == 1. { "yesterday".into() } else { format!("{days} days ago") }
}

/// A short age for the right edge of a row: "now", "40m", "3h", "2d"; nothing for no time.
pub fn age(at: Option<i64>, now: i64) -> String {
    let Some(at) = at.filter(|at| *at != 0) else { return String::new() };
    let minutes = (((now - at) as f64) / 60_000.).round().max(0.);
    if minutes < 1. {
        return "now".into();
    }
    if minutes < 60. {
        return format!("{minutes}m");
    }
    let hours = (minutes / 60.).round();
    if hours < 48. {
        return format!("{hours}h");
    }
    format!("{}d", (hours / 24.).round())
}

/// What a source's lamp says when pointed at: a line for each project it is set for ("shop: read 2 min ago as
/// sam-lee", "shop: twg isn't signed in", "shop: off"), or that it isn't read for any.
pub fn lamp_lines(health: &[Health], source: Source, projects: &HashMap<String, String>, now: i64) -> Vec<String> {
    let lines: Vec<String> = health
        .iter()
        .filter(|h| h.source == source.key())
        .map(|h| {
            let project = projects.get(&h.project_id).map_or("a project", String::as_str);
            match h.state.as_str() {
                "ok" => {
                    let who = h.account.as_ref().map(|account| format!(" as {account}")).unwrap_or_default();
                    format!("{project}: read {}{who}", ago(h.last_ok, now))
                }
                "error" => format!("{project}: {}", h.error.as_deref().unwrap_or("can't read")),
                _ => format!("{project}: {}", h.error.as_deref().unwrap_or("off")),
            }
        })
        .collect();
    if lines.is_empty() { vec![format!("{} isn't read for any project", source.name())] } else { lines }
}

/// What the status line says about a source ("GitHub read 2 min ago as sam-lee", "Jira can't be read"), and whether
/// that is something wrong.
pub fn status_words(health: &SourceHealth, source: Source, now: i64) -> (String, bool) {
    if health.state == "error" {
        return (format!("{} can't be read", source.name()), true);
    }
    let who = health.account.as_ref().map(|account| format!(" as {account}")).unwrap_or_default();
    (format!("{} read {}{who}", source.name(), ago(health.last_ok, now)), false)
}

/// A source that can't be read for a project, said first, above the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broken {
    pub source: Source,
    /// "Jira can't be read:" (with "for shop" when there are several projects).
    pub title: String,
    /// What is wrong, as a sentence: "twg isn't signed in."
    pub error: String,
    /// What to do, its commands between backticks: "Run `twg login --force`."
    pub fix: Option<String>,
    /// How old the data under it is, and when it is tried again.
    pub under: String,
}

/// " Trying again in 3 min.", " Trying again now.", or nothing.
fn next_try(at: Option<i64>, now: i64) -> String {
    let Some(at) = at.filter(|at| *at != 0) else { return String::new() };
    let minutes = (((at - now) as f64) / 60_000.).round();
    if minutes <= 0. { " Trying again now.".into() } else { format!(" Trying again in {minutes} min.") }
}

/// The lines for the sources that can't be read, a line for each project and source, in the order the core has them.
pub fn broken(health: &[Health], projects: &HashMap<String, String>, now: i64) -> Vec<Broken> {
    let many = health.iter().map(|h| h.project_id.as_str()).collect::<HashSet<_>>().len() > 1;
    health
        .iter()
        .filter(|h| h.state == "error")
        .filter_map(|h| {
            let source = Source::of(&h.source)?;
            let name = source.name();
            let project = projects.get(&h.project_id).map_or("a project", String::as_str);
            let title =
                if many { format!("{name} can't be read for {project}:") } else { format!("{name} can't be read:") };
            // the core starts its errors with the source's name; the title has said it
            let error = h.error.as_deref().unwrap_or("");
            let error =
                error.strip_prefix("GitHub:").or_else(|| error.strip_prefix("Jira:")).unwrap_or(error).trim_start();
            let from = match h.last_ok.filter(|at| *at != 0) {
                Some(at) => format!("What you see from {name} is from its last good read, {}.", ago(Some(at), now)),
                None => format!("Nothing from {name} has been read yet."),
            };
            Some(Broken {
                source,
                title,
                error: format!("{error}."),
                fix: h.fix.clone(),
                under: format!("{from}{}", next_try(h.next_try, now)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = enkidu_testkit::WORK_SAMPLE_NOW;

    fn sample() -> Snapshot {
        serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index reads as the core's Snapshot")
    }

    fn projects() -> HashMap<String, String> {
        [("p1".to_string(), "shop".to_string()), ("p2".to_string(), "tools".to_string())].into_iter().collect()
    }

    /// The sample reads here whole: what it holds is what the tests count on.
    #[test]
    fn the_sample_work_index_reads_as_the_cores_snapshot() {
        let work = sample();
        assert_eq!((work.health.len(), work.prs.len(), work.tickets.len()), (2, 9, 13));
        assert_eq!((work.sessions.len(), work.links.len(), work.items.len(), work.events.len()), (5, 16, 10, 7));
        assert_eq!(work.boards.len(), 3);
        assert!(work.board.is_none() && !work.checking);
        assert_eq!(work.prs[0].number, 2414);
        assert!(work.tickets.iter().any(|ticket| ticket.key == "SH-21309" && ticket.description.is_some()));
    }

    /// Marks a broken source's items stale: the source's part.
    #[test]
    fn a_sources_health_is_the_worst_of_its_projects() {
        let mut work = sample();
        let github = source_health(&work.health, Source::Github).unwrap();
        assert_eq!((github.state.as_str(), github.account.as_deref()), ("ok", Some("sam-lee")));
        assert_eq!(github.last_ok, Some(NOW - 2 * 60_000));
        assert!(github.broken.is_empty());
        for health in work.health.iter_mut().filter(|h| h.source == "jira") {
            health.state = "error".into();
            health.error = Some("Jira: twg isn't signed in".into());
        }
        let jira = source_health(&work.health, Source::Jira).unwrap();
        assert_eq!(
            (jira.state.as_str(), jira.error.as_deref(), jira.broken.len()),
            ("error", Some("Jira: twg isn't signed in"), 1)
        );
        assert_eq!(source_health(&[], Source::Github), None);
        // a source that is off for every project isn't read at all
        let off: Vec<Health> = work.health.iter().cloned().map(|h| Health { state: "off".into(), ..h }).collect();
        assert_eq!(source_health(&off, Source::Jira), None);
        // one project read, another not: the worst, the newest good read, and whoever it reads as
        let mut two = sample().health;
        two.push(Health {
            project_id: "p2".into(),
            source: "github".into(),
            state: "pending".into(),
            last_ok: None,
            last_try: None,
            error: None,
            fix: None,
            next_try: None,
            account: None,
        });
        let github = source_health(&two, Source::Github).unwrap();
        assert_eq!(
            (github.state.as_str(), github.account.as_deref(), github.last_ok),
            ("pending", Some("sam-lee"), Some(NOW - 2 * 60_000))
        );
    }

    #[test]
    fn a_lamp_is_hollow_when_off_amber_while_reading_else_as_the_source_stands() {
        let work = sample();
        let github = source_health(&work.health, Source::Github);
        assert_eq!(lamp(github.as_ref(), false), Lamp::Ok);
        assert_eq!(lamp(github.as_ref(), true), Lamp::Reading);
        assert_eq!(lamp(None, true), Lamp::Off, "a source that isn't read isn't being read either");
        let broken = SourceHealth { state: "error".into(), ..github.clone().unwrap() };
        assert_eq!(lamp(Some(&broken), false), Lamp::Error);
        let pending = SourceHealth { state: "pending".into(), ..github.unwrap() };
        assert_eq!(lamp(Some(&pending), false), Lamp::Pending);
        let names = [Lamp::Ok, Lamp::Error, Lamp::Pending, Lamp::Off, Lamp::Reading].map(Lamp::name);
        assert_eq!(names, ["ok", "error", "pending", "off", "reading"]);
    }

    /// Says ages short and long, and the rest of the two scales.
    #[test]
    fn says_ages_short_and_long() {
        assert_eq!(age(Some(NOW - 40 * 60_000), NOW), "40m");
        assert_eq!(age(Some(NOW - 3 * 3_600_000), NOW), "3h");
        assert_eq!(age(Some(NOW - 5 * 86_400_000), NOW), "5d");
        assert_eq!(ago(Some(NOW - 2 * 60_000), NOW), "2 min ago");
        assert_eq!(ago(None, NOW), "");
        // the rest of both
        assert_eq!(age(Some(NOW - 20_000), NOW), "now");
        assert_eq!(age(Some(NOW - 47 * 3_600_000), NOW), "47h");
        assert_eq!(age(None, NOW), "");
        assert_eq!(ago(Some(NOW - 30_000), NOW), "just now");
        assert_eq!(ago(Some(NOW - 3 * 3_600_000), NOW), "3 h ago");
        assert_eq!(ago(Some(NOW - 26 * 3_600_000), NOW), "yesterday");
        assert_eq!(ago(Some(NOW - 5 * 86_400_000), NOW), "5 days ago");
        assert_eq!(ago(Some(NOW + 60_000), NOW), "just now", "a clock a little ahead is now");
    }

    #[test]
    fn a_lamp_says_each_projects_read_and_the_status_line_the_newest() {
        let mut work = sample();
        assert_eq!(lamp_lines(&work.health, Source::Github, &projects(), NOW), ["shop: read 2 min ago as sam-lee"]);
        assert_eq!(lamp_lines(&[], Source::Jira, &projects(), NOW), ["Jira isn't read for any project"]);
        let github = source_health(&work.health, Source::Github).unwrap();
        assert_eq!(status_words(&github, Source::Github, NOW), ("GitHub read 2 min ago as sam-lee".to_string(), false));
        let jira = source_health(&work.health, Source::Jira).unwrap();
        assert_eq!(status_words(&jira, Source::Jira, NOW), ("Jira read 2 min ago as Sam Lee".to_string(), false));
        work.health[1].state = "error".into();
        work.health[1].error = Some("Jira: twg isn't signed in".into());
        assert_eq!(lamp_lines(&work.health, Source::Jira, &projects(), NOW), ["shop: Jira: twg isn't signed in"]);
        let jira = source_health(&work.health, Source::Jira).unwrap();
        assert_eq!(status_words(&jira, Source::Jira, NOW), ("Jira can't be read".to_string(), true));
        work.health[1].state = "off".into();
        work.health[1].error = None;
        assert_eq!(lamp_lines(&work.health, Source::Jira, &projects(), NOW), ["shop: off"]);
    }

    #[test]
    fn a_source_that_cant_be_read_says_what_is_wrong_what_to_do_and_how_old_its_data_is() {
        let mut work = sample();
        assert!(broken(&work.health, &projects(), NOW).is_empty());
        work.health[1].state = "error".into();
        work.health[1].error = Some("Jira: twg isn't signed in".into());
        work.health[1].fix = Some("Run `twg login --force`.".into());
        work.health[1].next_try = Some(NOW + 3 * 60_000);
        assert_eq!(
            broken(&work.health, &projects(), NOW),
            [Broken {
                source: Source::Jira,
                title: "Jira can't be read:".into(),
                error: "twg isn't signed in.".into(),
                fix: Some("Run `twg login --force`.".into()),
                under: "What you see from Jira is from its last good read, 2 min ago. Trying again in 3 min.".into(),
            }]
        );
        // never read, and due: it says so
        work.health[1].last_ok = None;
        work.health[1].next_try = Some(NOW - 1_000);
        work.health[1].fix = None;
        let line = &broken(&work.health, &projects(), NOW)[0];
        assert_eq!((line.under.as_str(), &line.fix), ("Nothing from Jira has been read yet. Trying again now.", &None));
        // with several projects each line names its own
        work.health.push(Health {
            project_id: "p2".into(),
            source: "github".into(),
            state: "error".into(),
            last_ok: None,
            last_try: None,
            error: Some("GitHub: gh isn't signed in".into()),
            fix: Some("Run `gh auth login`.".into()),
            next_try: None,
            account: None,
        });
        let lines = broken(&work.health, &projects(), NOW);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].title.as_str(), lines[1].title.as_str()),
            ("Jira can't be read for shop:", "GitHub can't be read for tools:")
        );
        assert_eq!(lines[1].under, "Nothing from GitHub has been read yet.");
    }
}
