//! Roblox Account Manager's local web API, plus the public item lookup.

use serde_json::Value;
use std::time::Duration;

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get(url: &str, query: &[(&str, &str)], secs: u64) -> Result<(u16, String), String> {
    let mut req = agent(secs).get(url);
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    let mut resp = req.call().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok((status, body))
}

fn base(port: u16) -> String {
    format!("http://localhost:{port}")
}

fn with_password<'a>(mut q: Vec<(&'a str, &'a str)>, password: &'a str) -> Vec<(&'a str, &'a str)> {
    if !password.is_empty() {
        q.push(("Password", password));
    }
    q
}

/// Explain a RAM reply that was not a success.
fn explain(status: u16, body: &str, setting: &str) -> String {
    match status {
        401 => format!("RAM refused ({body}). In RAM: Settings > Developer > enable '{setting}', and check the password."),
        _ => format!("RAM answered {status}: {body}"),
    }
}

pub fn is_running(port: u16) -> Result<(), String> {
    match get(&format!("{}/v2/Running", base(port)), &[], 3) {
        Ok(_) => Ok(()),
        Err(_) => Err(format!(
            "Roblox Account Manager is not answering on port {port}. Open RAM (the 'Nexus + WebServer' build) and turn on its web server."
        )),
    }
}

/// Account names saved in RAM.
pub fn accounts(port: u16, password: &str) -> Result<Vec<String>, String> {
    is_running(port)?;
    let q = with_password(vec![], password);
    let (status, body) = get(&format!("{}/v2/GetAccountsJson", base(port)), &q, 5)?;
    if status == 200 {
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("GetAccountsJson: {e}"))?;
        let list = v.get("Message").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
        let names: Vec<String> =
            list.iter().filter_map(|a| a.get("Username").and_then(Value::as_str).map(String::from)).collect();
        if !names.is_empty() || list.is_empty() {
            return Ok(names);
        }
    }
    let (status, body) = get(&format!("{}/v2/GetAccounts", base(port)), &q, 5)?;
    if status != 200 {
        return Err(explain(status, &body, "Allow GetAccounts"));
    }
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("Message").and_then(Value::as_str).map(String::from))
        .unwrap_or(body);
    Ok(msg.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
}

/// An account's sign-in cookie (needs "Allow GetCookie" in RAM).
pub fn cookie(port: u16, password: &str, account: &str) -> Result<String, String> {
    let q = with_password(vec![("Account", account)], password);
    let (status, body) = get(&format!("{}/v2/GetCookie", base(port)), &q, 5)?;
    if status != 200 {
        return Err(explain(status, &body, "Allow GetCookie"));
    }
    let text = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("Message").and_then(Value::as_str).map(String::from))
        .unwrap_or(body);
    Ok(text)
}

/// Name of a catalog item, or why it could not be found.
pub fn item_name(id: u64) -> Result<String, String> {
    let (status, body) = get(&format!("https://economy.roblox.com/v2/assets/{id}/details"), &[], 10)?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v.get("Name").and_then(Value::as_str).map(String::from).ok_or_else(|| "no such item".into())
}
