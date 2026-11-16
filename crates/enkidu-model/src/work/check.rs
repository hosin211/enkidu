//! "Check connections": every project's GitHub and Jira read now, then compared field by field with what `gh` and `twg`
//! say directly. Here is what each result says, in words; nothing is written to either.

use enkidu_core::work::Checked;

/// What the dialog says under its title.
pub const CHECK_SAYS: &str = "Reads GitHub and Jira now, then asks gh and twg directly and compares, field by field. \
                              Nothing is written to either.";
/// While a check runs.
pub const CHECK_RUNS: &str = "Reading GitHub and Jira, then asking gh and twg directly…";
/// When no project reads anything.
pub const CHECK_NOTHING: &str = "No project reads GitHub or Jira yet. Jira is turned on per project in its settings.";

/// One thing that differs between the index and the source itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DifferenceLine {
    /// What differs: `#2350` or `SH-21309`.
    pub thing: String,
    pub field: String,
    /// What the index has, "nothing" for nothing.
    pub index: String,
    /// What the source says, "nothing" for nothing.
    pub direct: String,
}

/// One source of one project, as its card in the dialog says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRow {
    /// github or jira.
    pub source: String,
    /// "GitHub" or "Jira".
    pub source_name: String,
    pub ok: bool,
    /// "GitHub for shop".
    pub title: String,
    /// "matches", "can't be read" or "2 differ".
    pub verdict: String,
    /// Why it can't be read, with what to run between backticks.
    pub error: Option<String>,
    /// What was compared, a line each.
    pub compared: Vec<String>,
    pub differences: Vec<DifferenceLine>,
}

impl CheckRow {
    pub fn of(checked: &Checked) -> CheckRow {
        let source_name = match checked.source.as_str() {
            "github" => "GitHub".to_string(),
            "jira" => "Jira".to_string(),
            other => other.to_string(),
        };
        let verdict = match (checked.ok, &checked.error) {
            (true, _) => "matches".to_string(),
            (false, Some(_)) => "can't be read".to_string(),
            (false, None) => format!("{} differ", checked.differences.len()),
        };
        let or_nothing = |said: &str| if said.is_empty() { "nothing".to_string() } else { said.to_string() };
        CheckRow {
            source: checked.source.clone(),
            title: format!("{source_name} for {}", checked.project),
            source_name,
            ok: checked.ok,
            verdict,
            error: checked.error.clone(),
            compared: checked.compared.clone(),
            differences: (checked.differences.iter())
                .map(|differs| DifferenceLine {
                    thing: differs.thing.clone(),
                    field: differs.field.clone(),
                    index: or_nothing(&differs.index),
                    direct: or_nothing(&differs.direct),
                })
                .collect(),
        }
    }

    /// A difference as one line: "#12 title: the index has Old, GitHub says New".
    pub fn difference_said(&self, differs: &DifferenceLine) -> String {
        format!(
            "{} {}: the index has {}, {} says {}",
            differs.thing, differs.field, differs.index, self.source_name, differs.direct
        )
    }

    /// All of the card in one text: its title and verdict, then the error, what was compared and each difference.
    pub fn said(&self) -> String {
        let mut lines = vec![format!("{}, {}", self.title, self.verdict)];
        lines.extend(self.error.iter().cloned());
        lines.extend(self.compared.iter().cloned());
        lines.extend(self.differences.iter().map(|differs| self.difference_said(differs)));
        lines.join("\n")
    }
}

/// The results as the dialog's cards, in the core's order (a project's GitHub, then its Jira).
pub fn check_rows(results: &[Checked]) -> Vec<CheckRow> {
    results.iter().map(CheckRow::of).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enkidu_core::work::live::Difference;

    fn checked(project: &str, source: &str, ok: bool, compared: &[&str], error: Option<&str>) -> Checked {
        Checked {
            project: project.into(),
            source: source.into(),
            ok,
            compared: compared.iter().map(|line| line.to_string()).collect(),
            differences: Vec::new(),
            error: error.map(String::from),
        }
    }

    /// Checks the connections: each source compared, a broken one says why, differences listed.
    #[test]
    fn each_source_is_compared_a_broken_one_says_why_and_differences_are_listed() {
        let differs =
            Difference { thing: "#12".into(), field: "title".into(), index: "Old".into(), direct: "New".into() };
        let rows = check_rows(&[
            checked("shop", "github", true, &["4 open PRs as `gh pr list --author @me` lists them"], None),
            checked("shop", "jira", false, &[], Some("twg isn't signed in. Run: twg login --force")),
            Checked { differences: vec![differs], ..checked("cafe", "github", false, &["1 open PRs"], None) },
        ]);
        let stands: Vec<String> = rows.iter().map(|row| format!("{} {}", row.source, row.ok)).collect();
        assert_eq!(stands, ["github true", "jira false", "github false"]);
        assert_eq!((rows[0].title.as_str(), rows[0].verdict.as_str()), ("GitHub for shop", "matches"));
        assert_eq!(rows[0].compared, ["4 open PRs as `gh pr list --author @me` lists them"]);
        assert_eq!((rows[1].title.as_str(), rows[1].verdict.as_str()), ("Jira for shop", "can't be read"));
        assert!(rows[1].error.as_deref().is_some_and(|error| error.contains("twg login --force")));
        assert_eq!(rows[2].verdict, "1 differ");
        assert_eq!(rows[2].difference_said(&rows[2].differences[0]), "#12 title: the index has Old, GitHub says New");
        assert_eq!(
            rows[2].said(),
            "GitHub for cafe, 1 differ\n1 open PRs\n#12 title: the index has Old, GitHub says New"
        );
    }

    /// A side that has nothing says "nothing".
    #[test]
    fn a_side_with_nothing_says_nothing() {
        let gone =
            Difference { thing: "SH-7".into(), field: "status".into(), index: String::new(), direct: "Done".into() };
        let none =
            Difference { thing: "SH-8".into(), field: "assignee".into(), index: "Sam".into(), direct: String::new() };
        let row = CheckRow::of(&Checked { differences: vec![gone, none], ..checked("shop", "jira", false, &[], None) });
        assert_eq!(row.verdict, "2 differ");
        assert_eq!(row.difference_said(&row.differences[0]), "SH-7 status: the index has nothing, Jira says Done");
        assert_eq!(row.difference_said(&row.differences[1]), "SH-8 assignee: the index has Sam, Jira says nothing");
    }
}
