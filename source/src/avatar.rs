//! Copy what a Roblox user is wearing: profile link or username in, item IDs out.
//! Uses Roblox's public avatar API, so no sign-in is needed.

use std::time::Duration;

pub struct Worn {
    pub user: String,
    pub items: Vec<u64>,
    /// Body parts, heads, animations and emotes that can't be tried on in CAC.
    pub skipped: usize,
}

#[derive(Debug, PartialEq)]
pub enum UserRef {
    Id(u64),
    Name(String),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// A profile link (`roblox.com/users/123/profile`), a user ID, or a username (with or without @).
pub fn parse_user(text: &str) -> Option<UserRef> {
    let s = text.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(i) = lower.find("/users/") {
        let d: String = lower[i + 7..].chars().take_while(|c| c.is_ascii_digit()).collect();
        return d.parse().ok().filter(|&n| n > 0).map(UserRef::Id);
    }
    if lower.contains("roblox.com") || lower.contains('/') {
        return None;
    }
    let name = s.trim_start_matches('@');
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        return name.parse().ok().filter(|&n| n > 0).map(UserRef::Id);
    }
    // Roblox usernames: 3-20 letters, digits and at most one underscore.
    if (3..=20).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Some(UserRef::Name(name.to_string()));
    }
    None
}

/// Asset types that CAC can try on from an item link.
fn wearable(asset_type: u64) -> bool {
    matches!(asset_type, 2 | 8 | 11 | 12 | 18 | 41..=47 | 64..=72)
}

fn json(resp: &mut ureq::http::Response<ureq::Body>) -> Result<serde_json::Value, String> {
    let text = resp.body_mut().read_to_string().map_err(|e| format!("download failed: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Roblox sent something unexpected: {e}"))
}

fn user_id(agent: &ureq::Agent, name: &str) -> Result<(u64, String), String> {
    let mut r = agent
        .post("https://users.roblox.com/v1/usernames/users")
        .header("Content-Type", "application/json")
        .send(serde_json::json!({ "usernames": [name], "excludeBannedUsers": false }).to_string())
        .map_err(|e| format!("could not reach Roblox: {e}"))?;
    if r.status().as_u16() != 200 {
        return Err(format!("Roblox didn't answer (HTTP {})", r.status().as_u16()));
    }
    let v = json(&mut r)?;
    let u = &v["data"][0];
    match u["id"].as_u64() {
        Some(id) => Ok((id, u["name"].as_str().unwrap_or(name).to_string())),
        None => Err(format!("there's no Roblox user called \"{name}\"")),
    }
}

fn user_name(agent: &ureq::Agent, id: u64) -> Result<String, String> {
    let mut r = agent
        .get(format!("https://users.roblox.com/v1/users/{id}"))
        .call()
        .map_err(|e| format!("could not reach Roblox: {e}"))?;
    match r.status().as_u16() {
        200 => Ok(json(&mut r)?["name"].as_str().unwrap_or("").to_string()),
        404 | 400 => Err(format!("there's no Roblox user with ID {id}")),
        s => Err(format!("Roblox didn't answer (HTTP {s})")),
    }
}

pub fn fetch(who: &UserRef) -> Result<Worn, String> {
    let agent = agent();
    let (id, user) = match who {
        UserRef::Id(id) => (*id, user_name(&agent, *id)?),
        UserRef::Name(n) => user_id(&agent, n)?,
    };
    let mut r = agent
        .get(format!("https://avatar.roblox.com/v1/users/{id}/avatar"))
        .call()
        .map_err(|e| format!("could not reach Roblox: {e}"))?;
    if r.status().as_u16() != 200 {
        return Err(format!("couldn't read {user}'s avatar (HTTP {})", r.status().as_u16()));
    }
    let v = json(&mut r)?;
    let mut items = Vec::new();
    let mut skipped = 0;
    for a in v["assets"].as_array().into_iter().flatten() {
        let (Some(aid), ty) = (a["id"].as_u64(), a["assetType"]["id"].as_u64().unwrap_or(0)) else { continue };
        if wearable(ty) {
            if !items.contains(&aid) {
                items.push(aid);
            }
        } else {
            skipped += 1;
        }
    }
    Ok(Worn { user, items, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_users() {
        assert_eq!(parse_user("https://www.roblox.com/users/156/profile"), Some(UserRef::Id(156)));
        assert_eq!(parse_user("roblox.com/users/1/profile?friendshipSource=1"), Some(UserRef::Id(1)));
        assert_eq!(parse_user("156"), Some(UserRef::Id(156)));
        assert_eq!(parse_user("@builderman"), Some(UserRef::Name("builderman".into())));
        assert_eq!(parse_user(" Some_User12 "), Some(UserRef::Name("Some_User12".into())));
        assert_eq!(parse_user("https://www.roblox.com/catalog/123/x"), None);
        assert_eq!(parse_user("a b"), None);
        assert_eq!(parse_user(""), None);
    }

    #[test]
    fn keeps_only_wearables() {
        assert!(wearable(8) && wearable(11) && wearable(12) && wearable(2) && wearable(41) && wearable(67));
        assert!(!wearable(17) && !wearable(27) && !wearable(48) && !wearable(61) && !wearable(79));
    }

    /// Hits the real Roblox API: `cargo test avatar -- --ignored`.
    #[test]
    #[ignore]
    fn copies_builderman() {
        let w = fetch(&UserRef::Name("builderman".into())).expect("fetch");
        println!("{} {:?} skipped {}", w.user, w.items, w.skipped);
        assert!(!w.items.is_empty());
        let w2 = fetch(&UserRef::Id(1)).expect("fetch id");
        assert_eq!(w2.user, "Roblox");
    }
}
