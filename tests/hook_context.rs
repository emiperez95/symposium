//! One `cargo agents hook` invocation answers for many sources: symposium's
//! own output (here, the notice that `crate-a` ships a plugin awaiting
//! consent) and every plugin hook. Each one's context must reach the agent,
//! in dispatch order, the way agents themselves combine several hooks.
//!
//! `first` and `second` add context; `zz-silent` runs last and adds none.

use serde_json::Value;
use symposium::hook_schema::{HookAgent, agent_event};
use symposium_testlib::{HookStep, TestContext, TestMode, with_fixture};

/// `step`'s stdout for `agent`, invoked the way the agent would.
async fn respond(ctx: &TestContext, agent: HookAgent, step: HookStep) -> String {
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
    let stdout = ctx.invoke_hook(agent, event, &payload).await.unwrap();
    String::from_utf8(stdout.stdout).unwrap()
}

/// Assert `stdout` holds every one of `parts`, in this order.
fn assert_in_order(agent: HookAgent, stdout: &str, parts: &[&str]) {
    let mut from = 0;
    for part in parts {
        let Some(at) = stdout[from..].find(part) else {
            panic!("{agent:?}: expected {part:?} after byte {from}, got: {stdout}");
        };
        from += at + part.len();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_start_keeps_symposium_and_plugin_context() {
    with_fixture(TestMode::SimulationOnly, &["context-merge0"], async |ctx| {
        for agent in [
            HookAgent::Claude,
            HookAgent::Codex,
            HookAgent::Copilot,
            HookAgent::Kiro,
            HookAgent::Antigravity,
        ] {
            let stdout = respond(&ctx, agent, HookStep::session_start()).await;
            assert_in_order(
                agent,
                &stdout,
                &["crate-a", "first plugin context", "second plugin context"],
            );
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Antigravity is left out: it reads no context on `PostToolUse` at all.
#[tokio::test(flavor = "multi_thread")]
async fn post_tool_use_keeps_every_plugin_context() {
    with_fixture(TestMode::SimulationOnly, &["context-merge0"], async |ctx| {
        let step = || HookStep::PostToolUse {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "cargo build"}),
            tool_response: serde_json::json!({"stdout": "ok"}),
        };
        for agent in [
            HookAgent::Claude,
            HookAgent::Codex,
            HookAgent::Copilot,
            HookAgent::Kiro,
        ] {
            let stdout = respond(&ctx, agent, step()).await;
            assert_in_order(
                agent,
                &stdout,
                &["first post context", "second post context"],
            );
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// A symposium-format hook that prints valid JSON that is not an object
/// (here a bare string) is ignored. Merged as-is it would replace the whole
/// response; on `PreToolUse` no joined context is re-applied over it.
#[tokio::test(flavor = "multi_thread")]
async fn non_object_output_does_not_replace_the_response() {
    with_fixture(
        TestMode::SimulationOnly,
        &["non-object-output0"],
        async |ctx| {
            for agent in [HookAgent::Claude, HookAgent::Codex, HookAgent::Copilot] {
                let stdout = respond(
                    &ctx,
                    agent,
                    HookStep::PreToolUse {
                        tool_name: "Bash".to_string(),
                        tool_input: serde_json::json!({"command": "ls"}),
                    },
                )
                .await;
                assert!(!stdout.contains("not an object"), "{agent:?}: {stdout}");
            }
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// A hook in the host agent's own format receives the agent's payload byte
/// for byte, including fields Symposium does not model.
#[tokio::test(flavor = "multi_thread")]
async fn native_hook_receives_the_payload_unchanged() {
    with_fixture(TestMode::SimulationOnly, &["native-input0"], async |ctx| {
        let workspace = ctx.workspace_root.clone().unwrap();
        let handler =
            agent_event(HookAgent::Antigravity, HookStep::session_start().event()).unwrap();
        let mut payload: Value = serde_json::from_str(
            &handler
                .translate_input(
                    &HookStep::session_start().to_input_event(&workspace.display().to_string()),
                )
                .to_string()
                .unwrap(),
        )
        .unwrap();
        payload["unmodeledField"] = Value::from("kept");
        ctx.invoke_hook(
            HookAgent::Antigravity,
            HookStep::session_start().event(),
            &payload,
        )
        .await
        .unwrap();
        let received = std::fs::read_to_string(workspace.join("received.json"))?;
        assert_eq!(received, serde_json::to_string(&payload)?);
        Ok(())
    })
    .await
    .unwrap();
}
