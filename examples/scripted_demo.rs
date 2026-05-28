//! Scripted-mode usage example.
//!
//! Demonstrates the library API downstream callers can use to build a
//! deterministic test fixture for agent loops:
//!
//! 1. Construct a `Script` programmatically, or load one from JSON.
//! 2. Walk it via `Script::next_turn()` — the same API the HTTP
//!    handlers use under the hood — and show what each turn produces.
//! 3. Reload from disk to confirm the JSON wire format round-trips.
//!
//! Run with: cargo run --example scripted_demo
//!
//! This example does not boot the HTTP server — that's exercised via
//! `tests/scripted_test.rs` (integration tests against the live router)
//! and the CI `examples` job (smoke-tests the server with the bundled
//! script file via curl).

use llmsim::{
    script::{auto_tool_call_id, ScriptError, ScriptedResponse},
    OnExhausted, Script, SimError, SimToolCall, SimTurn,
};
use serde_json::json;

fn main() -> Result<(), ScriptError> {
    println!("=== LLMSim Scripted Mode Demo ===\n");

    // 1. Build a script in code (the shape a downstream test would use).
    println!("1. Programmatic Script");
    println!("----------------------");
    let script = Script::new(vec![
        SimTurn::ToolCalls {
            calls: vec![SimToolCall {
                name: "bash".into(),
                arguments: json!({"command": "echo hello > /tmp/x.txt"}),
                id: None,
            }],
        },
        SimTurn::ToolCalls {
            calls: vec![SimToolCall {
                name: "bash".into(),
                arguments: json!({"command": "sed -i s/hello/world/ /tmp/x.txt"}),
                id: None,
            }],
        },
        SimTurn::Mixed {
            text: "All done.".into(),
            calls: vec![SimToolCall {
                name: "bash".into(),
                arguments: json!({"command": "cat /tmp/x.txt"}),
                id: Some("call_verify".into()),
            }],
        },
        SimTurn::Assistant {
            text: "done".into(),
        },
        SimTurn::Error(SimError::RateLimit),
    ])
    .with_on_exhausted(OnExhausted::Error);

    walk(&script);

    // 2. Round-trip via the JSON file shipped in examples/.
    println!("\n2. Load from disk (examples/scripted_demo.json)");
    println!("-----------------------------------------------");
    let from_disk = Script::from_file("examples/scripted_demo.json")?;
    println!(
        "loaded {} turns, on_exhausted={:?}",
        from_disk.len(),
        from_disk.on_exhausted()
    );
    walk(&from_disk);

    // 3. Auto-generated tool-call ids are stable across the script.
    println!("\n3. Auto-generated tool call ids");
    println!("-------------------------------");
    for turn in 0..3 {
        for call in 0..2 {
            println!(
                "  turn={} call={} -> {}",
                turn,
                call,
                auto_tool_call_id(turn, call)
            );
        }
    }

    println!("\nNext steps:");
    println!("  - Boot the server: cargo run -- serve --config examples/scripted_demo.toml");
    println!("  - Drive it via any OpenAI-compatible client (see specs/scripted-mode.md).");

    Ok(())
}

fn walk(script: &Script) {
    for i in 0..(script.len() + 1) {
        match script.next_turn() {
            ScriptedResponse::Turn(SimTurn::Assistant { text }) => {
                println!("  [{}] assistant: {:?}", i, text);
            }
            ScriptedResponse::Turn(SimTurn::ToolCalls { calls }) => {
                println!("  [{}] tool_calls:", i);
                for c in calls {
                    println!("        {}({})", c.name, c.arguments);
                }
            }
            ScriptedResponse::Turn(SimTurn::Mixed { text, calls }) => {
                println!("  [{}] mixed: text={:?}", i, text);
                for c in calls {
                    println!("        {}({})", c.name, c.arguments);
                }
            }
            ScriptedResponse::Turn(SimTurn::Error(err)) => {
                println!(
                    "  [{}] error: {} (HTTP {})",
                    i,
                    err.message(),
                    err.status_code()
                );
            }
            ScriptedResponse::Exhausted => {
                println!("  [{}] EXHAUSTED (on_exhausted=Error)", i);
            }
        }
    }
}
