//! Pure helpers for reading what the agent printed and logged.

use regex::Regex;

/// Answer lines in a Claude pane capture. An answer starts at a line with the
/// `⏺` marker; its indented continuation lines belong to it until the next
/// prompt echo (`❯`) or rule (`─`). The prompt echo carries the prompt's own
/// words, so matching the whole pane would match the question, not the answer.
pub fn claude_answer_blocks(pane: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut in_answer = false;
    for line in pane.lines() {
        if let Some(rest) = line.strip_prefix('⏺') {
            blocks.push(rest.trim().to_string());
            in_answer = true;
        } else if in_answer {
            let t = line.trim_start();
            if t.starts_with('❯') || t.starts_with('─') || t.starts_with('✻') {
                in_answer = false;
            } else if line.starts_with("  ") {
                let b = blocks.last_mut().unwrap();
                b.push('\n');
                b.push_str(line.trim());
            } else if line.trim().is_empty() {
                // blank lines separate paragraphs of one answer
            } else {
                in_answer = false;
            }
        }
    }
    blocks
}

pub fn count_matches(re: &Regex, text: &str) -> usize {
    text.lines().filter(|l| re.is_match(l)).count()
}

/// Outcome of a log expectation: `count = None` on `expect` means "at least
/// one", on `reject` means "none"; an explicit count means exactly that many.
pub fn check_log(re: &Regex, log: &str, count: Option<usize>, reject: bool) -> (bool, String) {
    let n = count_matches(re, log);
    let (ok, want) = match (count, reject) {
        (Some(c), _) => (n == c, format!("exactly {c}")),
        (None, false) => (n >= 1, "at least 1".to_string()),
        (None, true) => (n == 0, "none".to_string()),
    };
    (ok, format!("{n} match(es), wanted {want}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: &str = "\
❯ Answer SKILL-VISIBLE or SKILL-MISSING
  on the first line

⏺ SKILL-MISSING

  The skills I can see:
  - dataviz
  - loop

✻ Churned for 2s
────────────────
❯
";

    #[test]
    fn answer_excludes_prompt_echo() {
        let blocks = claude_answer_blocks(PANE);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].starts_with("SKILL-MISSING"));
        assert!(blocks[0].contains("- loop"));
        assert!(!blocks[0].contains("SKILL-VISIBLE"));
    }

    #[test]
    fn counts_each_answer_separately() {
        let two = format!("{PANE}\n⏺ second\n");
        assert_eq!(claude_answer_blocks(&two).len(), 2);
    }

    #[test]
    fn log_checks() {
        let re = Regex::new("requested reloadSkills").unwrap();
        let log = "a\nrequested reloadSkills\nb\n";
        assert!(check_log(&re, log, None, false).0);
        assert!(check_log(&re, log, Some(1), false).0);
        assert!(!check_log(&re, log, Some(2), false).0);
        assert!(!check_log(&re, log, None, true).0);
        assert!(check_log(&re, "nothing", None, true).0);
    }
}
