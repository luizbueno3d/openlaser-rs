use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_mcp_initialize_lists_tools_and_recovers_parse_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_openlaser-cli"))
        .args(["mcp", "--automation-read-root"])
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"not-json\n").unwrap();
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n").unwrap();
    stdin
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"2026-07-28\"},\"params\":{}}\n",
        )
        .unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"session_list\",\"arguments\":{\"force\":true}}}\n")
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.first().and_then(|v| v.pointer("/error/code")).and_then(serde_json::Value::as_i64), Some(-32700));
    assert_eq!(lines.get(1).and_then(|v| v.pointer("/result/serverInfo/name")).and_then(serde_json::Value::as_str), Some("openlaser"));
    let listed = lines.get(2).and_then(|v| v.pointer("/result/tools")).and_then(serde_json::Value::as_array).unwrap();
    assert!(listed.iter().any(|tool| tool.get("name").and_then(serde_json::Value::as_str) == Some("stop")));
    assert_eq!(lines.get(2).and_then(|v| v.pointer("/result/resultType")).and_then(serde_json::Value::as_str), Some("complete"));
    assert_eq!(lines.get(3).and_then(|v| v.pointer("/error/code")).and_then(serde_json::Value::as_i64), Some(-32602));
    assert!(lines.get(3).and_then(|v| v.pointer("/error/message")).and_then(serde_json::Value::as_str).unwrap().contains("force"));
}
