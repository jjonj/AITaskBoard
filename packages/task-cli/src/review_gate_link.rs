//! Best-effort DISPLAY-ONLY TaskBoard -> OpenRig gate link registration.
//! This path never releases a gate. Only independently authenticated T3 MAIN can.
use std::{io::{Read, Write}, net::{SocketAddr, TcpStream}, path::Path, time::Duration};
use serde_json::{json, Value};

fn encode(value: &str) -> String {
    let mut s = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { s.push(b as char); }
        else { s.push_str(&format!("%{b:02X}")); }
    }
    s
}

pub fn register(root: &Path, task_id: &str, manifest_revision: &str, gate: &str, title: &str) -> Value {
    if !gate.starts_with("qitem-") || !gate[6..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return json!({"registered":false,"error":"invalid_gate_id"});
    }
    let Ok(bearer) = std::env::var("OPENRIG_AUTH_BEARER_TOKEN") else {
        return json!({"registered":false,"error":"openrig_bearer_not_configured"});
    };
    if bearer.is_empty() || bearer.bytes().any(|b| b == b'\r' || b == b'\n') {
        return json!({"registered":false,"error":"invalid_openrig_bearer"});
    }
    let url = format!("taskboard://review/{task_id}?root={}", encode(&root.to_string_lossy()));
    let data = json!({"version":1,"gateQitemId":gate,"taskId":task_id,"reviewId":task_id,
        "reviewRevision":manifest_revision,"reviewUrl":url,"title":title}).to_string();
    let addr: SocketAddr = "127.0.0.1:7433".parse().expect("constant address");
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return json!({"registered":false,"error":"openrig_unreachable"});
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let request = format!("POST /api/integrations/taskboard-review-gates/links HTTP/1.1\r\nHost: 127.0.0.1:7433\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}", data.len(), data);
    if stream.write_all(request.as_bytes()).is_err() { return json!({"registered":false,"error":"openrig_write_failed"}); }
    let mut response = Vec::new();
    if stream.take(131_072).read_to_end(&mut response).is_err() { return json!({"registered":false,"error":"openrig_response_failed"}); }
    let text = String::from_utf8_lossy(&response);
    let ok = text.starts_with("HTTP/1.1 200 ") || text.starts_with("HTTP/1.0 200 ");
    let body = text.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or("");
    let receipt: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({"error":"non_json_response"}));
    if ok && receipt.get("registered").and_then(Value::as_bool) == Some(true) {
       return json!({"registered":true,"status":receipt["status"],"releaseAuthority":"desktop-main-operator-only"});
    }
    json!({"registered":false,"error":receipt.get("error").and_then(Value::as_str).unwrap_or("openrig_link_rejected")})
}

#[cfg(test)] mod tests {
   use super::encode;
   #[test] fn root_parameter_is_url_encoded() {
      assert_eq!(encode(r"B:\GDrive\Misc Tasks"), "B%3A%5CGDrive%5CMisc%20Tasks");
   }
}