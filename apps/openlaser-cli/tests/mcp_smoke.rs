//! The one spawned smoke test of this binary (§6.1): everything else runs in process through `src/lib.rs`.
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_mcp_initialize_lists_tools() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_openlaser-cli"))
        .args(["mcp", "--automation-read-root"])
        .arg(&root)
        .env("OPENLASER_NO_HW", "1")
        .env_remove("OPENLASER_HW_PRESENT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"not-json\n").unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"clientInfo\":{\"name\":\"smoke\",\"version\":\"1\"}}}\n")
        .unwrap();
    stdin
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"2026-07-28\"}}}\n",
        )
        .unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"session_list\",\"arguments\":{\"force\":true}}}\n")
        .unwrap();
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n").unwrap();
    // An oversized line gets an error reply and ends the session; nothing after it is served.
    let oversized = vec![b' '; openlaser_control::budgets::MAX_REQUEST_BYTES + 1];
    let _ = stdin.write_all(&oversized);
    let _ = stdin.write_all(b"\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n");
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "an oversized line ends the session with an error");
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 6, "{text}");
    // Tool calls run on worker threads, so replies are matched by id, not by order.
    let by_id = |id: serde_json::Value| lines.iter().find(|line| line.get("id") == Some(&id)).unwrap_or_else(|| panic!("no reply {id}: {text}"));
    let nulls: Vec<_> = lines.iter().filter(|line| line.get("id") == Some(&serde_json::Value::Null)).collect();
    assert_eq!(nulls.len(), 2, "{text}");
    assert_eq!(nulls[0].pointer("/error/code").and_then(serde_json::Value::as_i64), Some(-32700));
    assert!(nulls[1].pointer("/error/message").and_then(serde_json::Value::as_str).unwrap().contains("exceeds"));
    let init = by_id(serde_json::json!(1));
    assert_eq!(init.pointer("/result/serverInfo/name").and_then(serde_json::Value::as_str), Some("openlaser"));
    assert_eq!(init.pointer("/result/protocolVersion").and_then(serde_json::Value::as_str), Some("2025-06-18"));
    let list = by_id(serde_json::json!(2));
    let listed = list.pointer("/result/tools").and_then(serde_json::Value::as_array).unwrap();
    assert!(listed.iter().any(|tool| tool.get("name").and_then(serde_json::Value::as_str) == Some("stop")));
    assert_eq!(list.pointer("/result/resultType").and_then(serde_json::Value::as_str), Some("complete"));
    let refused = by_id(serde_json::json!(3));
    assert_eq!(refused.pointer("/error/code").and_then(serde_json::Value::as_i64), Some(-32602));
    assert!(refused.pointer("/error/message").and_then(serde_json::Value::as_str).unwrap().contains("force"));
    assert!(by_id(serde_json::json!(4)).get("result").is_some());
    assert!(lines.iter().all(|line| line.get("id") != Some(&serde_json::json!(5))), "a line after the oversized one was served");
}
