//! 1 Kopper K1: `simulation_enabled` decides whether the server serves the
//! simulation toolset. Off by default: the toolset is not listed or active,
//! cannot be loaded or unloaded (the error names the setting, in the single and
//! the array form), its tools are unknown and never in `tools/list` (also with
//! `eager_toolsets` and `auto_load_toolsets`), and the process carries no
//! simulation runtime. On: listed, loadable, and the runtime is there. Each
//! test starts the real binary with a private `HOME`.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// One server session in `dir` (also its `HOME` and state folder) with
/// `config` as its `konnect.toml`; returns the result of each tool call, then
/// the result of a final `tools/list`. A JSON-RPC error stands as a tool error
/// with its message.
fn session(dir: &Path, config: &str, calls: &[(&str, Value)]) -> Vec<Value> {
    std::fs::write(dir.join("konnect.toml"), config).unwrap();
    let mut requests = vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "simulation-switch-test", "version": "0"}}}),
    ];
    for (n, (tool, arguments)) in calls.iter().enumerate() {
        requests.push(
            json!({"jsonrpc": "2.0", "id": 100 + n, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}}),
        );
    }
    let list_id = 100 + calls.len();
    requests.push(json!({"jsonrpc": "2.0", "id": list_id, "method": "tools/list", "params": {}}));
    let mut child = Command::new(env!("CARGO_BIN_EXE_konnect"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("KONNECT_STATE_DIR", dir.join("state"))
        .env("XDG_CONFIG_HOME", dir.join(".config"))
        .env("KONNECT_CALL_LOG", "off")
        .env_remove("KICAD_API_SOCKET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("konnect starts");
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in &requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());
    let mut results = vec![Value::Null; calls.len() + 1];
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if let Some(id) = message["id"].as_u64().filter(|id| *id >= 100) {
            results[(id - 100) as usize] = message.get("result").cloned().unwrap_or_else(|| {
                json!({"isError": true, "content": [{"type": "text",
                    "text": message["error"]["message"].as_str().unwrap_or_default()}]})
            });
        }
    }
    child.wait().unwrap();
    results
}

fn text_of(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn body(result: &Value) -> Value {
    serde_json::from_str(&text_of(result)).unwrap_or(Value::Null)
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap_or_else(|| panic!("a list: {list:#}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

/// Every simulation tool's name, from the registry.
fn simulation_tools() -> Vec<&'static str> {
    konnect_core::router::registry::tools_for("simulation")
        .expect("registered")
        .iter()
        .map(|tool| tool.name)
        .collect()
}

fn lists_no_simulation_tool(list: &Value) -> bool {
    let listed = names(&list["tools"]);
    simulation_tools()
        .iter()
        .all(|tool| !listed.iter().any(|name| name == tool))
}

fn calls() -> Vec<(&'static str, Value)> {
    vec![
        ("list_toolboxes", json!({})),
        ("load_toolset", json!({"name": "simulation"})),
        ("get_installation_info", json!({})),
        // a simulation tool, called by name without loading its toolset
        ("check_simulation_readiness", json!({})),
        // the array form: the served toolset loads, the hidden one is refused
        (
            "load_toolset",
            json!({"name": ["pcb_routing", "simulation"]}),
        ),
        ("get_active_toolsets", json!({})),
        ("unload_toolset", json!({"name": "simulation"})),
    ]
}

const SIMULATION: &str = "simulation";

#[test]
fn simulation_is_off_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let r = session(dir.path(), "", &calls());
    let listed = names(&body(&r[0])["toolsets"]);
    assert!(listed.contains(&"pcb_routing".to_string()), "{listed:?}");
    assert!(!listed.contains(&SIMULATION.to_string()), "{listed:?}");

    assert_eq!(r[1]["isError"], json!(true), "{:#}", r[1]);
    assert!(
        text_of(&r[1]).contains("simulation_enabled = true"),
        "{:#}",
        r[1]
    );

    assert_eq!(
        body(&r[2])["ngspice"]["probe_status"],
        "not_available",
        "{:#}",
        r[2]
    );

    assert_eq!(r[3]["isError"], json!(true), "{:#}", r[3]);
    assert!(
        !text_of(&r[3]).contains("load_toolset('simulation')"),
        "{:#}",
        r[3]
    );

    let batch = text_of(&r[4]);
    assert!(batch.contains("pcb_routing"), "{batch}");
    assert!(batch.contains("simulation_enabled = true"), "{batch}");
    let active = names(&body(&r[5])["active_toolsets"]);
    assert!(active.contains(&"pcb_routing".to_string()), "{active:?}");
    assert!(!active.contains(&SIMULATION.to_string()), "{active:?}");

    assert_eq!(r[6]["isError"], json!(true), "{:#}", r[6]);
    assert!(
        text_of(&r[6]).contains("simulation_enabled = true"),
        "{:#}",
        r[6]
    );

    assert!(lists_no_simulation_tool(&r[7]), "{:#}", r[7]);
}

/// Off, with every toolset loaded at startup and auto-load on: `tools/list`
/// carries no simulation tool, and a call by name does not load it back.
#[test]
fn simulation_stays_off_with_eager_and_auto_load() {
    let dir = tempfile::tempdir().unwrap();
    let r = session(
        dir.path(),
        "eager_toolsets = true\nauto_load_toolsets = true\n",
        &[
            ("check_simulation_readiness", json!({})),
            ("get_active_toolsets", json!({})),
        ],
    );
    assert_eq!(r[0]["isError"], json!(true), "{:#}", r[0]);
    let active = names(&body(&r[1])["active_toolsets"]);
    assert!(active.contains(&"pcb_routing".to_string()), "{active:?}");
    assert!(!active.contains(&SIMULATION.to_string()), "{active:?}");
    assert!(
        names(&r[2]["tools"]).contains(&"route_trace".to_string()),
        "eager loads the rest"
    );
    assert!(lists_no_simulation_tool(&r[2]), "{:#}", r[2]);
}

#[test]
fn simulation_is_served_when_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let r = session(dir.path(), "simulation_enabled = true\n", &calls());
    assert!(names(&body(&r[0])["toolsets"]).contains(&SIMULATION.to_string()));
    assert_ne!(r[1]["isError"], json!(true), "{:#}", r[1]);
    assert!(body(&r[1])["tools_added"].as_u64().unwrap() > 0);
    assert_ne!(
        body(&r[2])["ngspice"]["probe_status"],
        "not_available",
        "{:#}",
        r[2]
    );
    assert!(names(&body(&r[5])["active_toolsets"]).contains(&SIMULATION.to_string()));
    assert_ne!(
        r[6]["isError"],
        json!(true),
        "unloading a served toolset works: {:#}",
        r[6]
    );
}
