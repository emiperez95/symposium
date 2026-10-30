//! Claude Code discovers skills before `SessionStart` hooks finish, so a skill
//! that symposium's `SessionStart` auto-sync installs stays invisible to the
//! session unless the hook answers `hookSpecificOutput.reloadSkills: true`.
//!
//! `native` is a Claude-format plugin hook that answers `reloadSkills: false`;
//! `portable` is a symposium-format one that says nothing about reloading.
//! Neither may cancel symposium's request.

use serde_json::Value;
use symposium::config::AgentEntry;
use symposium::hook_schema::{HookAgent, agent_event};
use symposium_testlib::{HookStep, TestContext, TestMode, with_fixture};

const FIXTURES: &[&str] = &["plugins0", "workspace0", "session-reload0"];
/// Configure `names` as the user's agents, so sync installs skills for them.
fn set_agents(ctx: &mut TestContext, names: &[&str]) {
    ctx.sym.config.agents = names
        .iter()
        .map(|name| AgentEntry {
            name: name.to_string(),
        })
        .collect();
}

/// Whether sync installed the fixture's `serde-guidance` skill for Claude.
fn skill_installed(ctx: &TestContext) -> bool {
    let skills = ctx.workspace_root.as_ref().unwrap().join(".claude/skills");
    std::fs::read_dir(skills).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path().join("SKILL.md").is_file()
                && e.file_name()
                    .to_string_lossy()
                    .starts_with("serde-guidance")
        })
    })
}

/// `hookSpecificOutput` of Claude's `SessionStart` answer.
async fn claude_specific(ctx: &TestContext) -> (Value, String) {
    let stdout = session_start(ctx, HookAgent::Claude).await;
    let output: Value = serde_json::from_str(&stdout).unwrap();
    (output["hookSpecificOutput"].clone(), stdout)
}

/// `SessionStart`'s stdout for `agent`, invoked the way the agent would.
async fn session_start(ctx: &TestContext, agent: HookAgent) -> String {
    let step = HookStep::session_start();
    let event = step.event();
    let cwd = ctx.workspace_root.as_ref().unwrap().display().to_string();
    let handler = agent_event(agent, event).expect("agent handles the event");
    let payload: Value = serde_json::from_str(
        &handler
            .translate_input(&step.to_input_event(&cwd))
            .to_string()
            .unwrap(),
    )
    .unwrap();
    let response = ctx.invoke_hook(agent, event, &payload).await.unwrap();
    String::from_utf8(response.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_session_start_requests_skill_reload_after_sync() {
    with_fixture(TestMode::SimulationOnly, FIXTURES, async |mut ctx| {
        set_agents(&mut ctx, &["claude"]);
        let (specific, stdout) = claude_specific(&ctx).await;
        assert!(skill_installed(&ctx), "sync should install the skill");
        assert_eq!(specific["reloadSkills"], true, "{stdout}");
        assert_eq!(specific["hookEventName"], "SessionStart", "{stdout}");
        let context = specific["additionalContext"].as_str().unwrap_or_default();
        assert!(context.contains("native plugin context"), "{stdout}");
        assert!(context.contains("portable plugin context"), "{stdout}");
        Ok(())
    })
    .await
    .unwrap();
}

/// `reloadSkills` is Claude Code's field; no other agent receives it.
#[tokio::test(flavor = "multi_thread")]
async fn other_agents_never_receive_reload_skills() {
    with_fixture(TestMode::SimulationOnly, FIXTURES, async |ctx| {
        for agent in [
            HookAgent::Codex,
            HookAgent::Copilot,
            HookAgent::Kiro,
            HookAgent::Antigravity,
        ] {
            let stdout = session_start(&ctx, agent).await;
            assert!(!stdout.contains("reloadSkills"), "{agent:?}: {stdout}");
            assert!(
                stdout.contains("portable plugin context"),
                "{agent:?}: {stdout}"
            );
        }
        Ok(())
    })
    .await
    .unwrap();
}
