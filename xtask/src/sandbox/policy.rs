//! The `srt` settings file.

use serde_json::{Value, json};
use std::path::Path;

/// Paths under the real HOME the sandbox must not read: the user's own agent
/// state, Symposium state and credentials.
const DENY_READ: &[&str] = &[
    ".claude",
    ".claude-work",
    ".claude.json",
    ".symposium",
    ".ssh",
    ".aws",
    ".config",
];

pub fn srt_settings(real_home: &Path, run: &Path, tmp: &Path, extra_domains: &[String]) -> Value {
    let mut domains = vec!["api.anthropic.com".to_string()];
    for d in extra_domains {
        if !domains.contains(d) {
            domains.push(d.clone());
        }
    }
    let deny_read: Vec<String> = DENY_READ
        .iter()
        .map(|p| real_home.join(p).display().to_string())
        .collect();
    json!({
        // Without a pty the seatbelt profile blocks the terminal ioctl, the agent
        // cannot enter raw mode, and keys are echoed instead of delivered.
        "allowPty": true,
        "network": { "allowedDomains": domains, "deniedDomains": [] },
        "filesystem": {
            "denyRead": deny_read,
            "allowWrite": [run.display().to_string(), tmp.display().to_string()],
            "denyWrite": [],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_shape() {
        let v = srt_settings(
            Path::new("/Users/me"),
            Path::new("/private/tmp/x/run"),
            Path::new("/private/tmp/x/tmp"),
            &["example.com".into(), "api.anthropic.com".into()],
        );
        assert_eq!(v["allowPty"], true);
        assert_eq!(
            v["network"]["allowedDomains"],
            json!(["api.anthropic.com", "example.com"])
        );
        let deny = v["filesystem"]["denyRead"].as_array().unwrap();
        assert!(deny.contains(&json!("/Users/me/.claude.json")));
        assert!(deny.contains(&json!("/Users/me/.symposium")));
        assert_eq!(
            v["filesystem"]["allowWrite"],
            json!(["/private/tmp/x/run", "/private/tmp/x/tmp"])
        );
    }
}
