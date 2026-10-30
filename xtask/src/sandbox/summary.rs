use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Versions {
    pub agent: String,
    pub srt: String,
    pub symposium_commit: String,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub scenario: String,
    pub variant: String,
    pub label: String,
    pub agent: String,
    pub started_at: String,
    pub versions: Versions,
    pub checks: Vec<Check>,
    pub verdict: String,
}

impl Summary {
    pub fn passed(&self) -> bool {
        self.verdict == "pass"
    }
}

pub fn verdict_of(checks: &[Check]) -> &'static str {
    if !checks.is_empty() && checks.iter().all(|c| c.passed) {
        "pass"
    } else {
        "fail"
    }
}

/// `PASS scenario/variant (3/3) <run>` or `FAIL ... [failed checks]`.
pub fn verdict_line(s: &Summary, run: &std::path::Path) -> String {
    let ok = s.checks.iter().filter(|c| c.passed).count();
    let mut line = format!(
        "{} {}/{} ({}/{})",
        s.verdict.to_uppercase(),
        s.scenario,
        s.variant,
        ok,
        s.checks.len()
    );
    let failed: Vec<&str> = s
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    if !failed.is_empty() {
        line.push_str(&format!(" failed: {}", failed.join("; ")));
    }
    line.push_str(&format!("  {}", run.display()));
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, passed: bool) -> Check {
        Check {
            name: name.into(),
            passed,
            detail: String::new(),
        }
    }

    #[test]
    fn verdicts() {
        assert_eq!(verdict_of(&[c("a", true), c("b", true)]), "pass");
        assert_eq!(verdict_of(&[c("a", true), c("b", false)]), "fail");
        assert_eq!(verdict_of(&[]), "fail");
    }
}
