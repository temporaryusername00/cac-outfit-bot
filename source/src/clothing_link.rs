//! Look up a classic Shirt, Pants or T-Shirt from a catalog link and download its template image.
//!
//! A clothing asset is a small Roblox model whose ShirtTemplate / PantsTemplate / Graphic points at
//! an image asset. Older items download without signing in; newer ones need a signed-in request,
//! so the saved accounts are tried in turn (their cookies go only to roblox.com).

use std::time::Duration;

use image::RgbaImage;

use crate::clothing::Kind;

pub struct Item {
    pub id: u64,
    /// None when the link was an image or decal rather than a clothing item.
    pub kind: Option<Kind>,
    pub name: String,
    pub creator: String,
    pub template_id: u64,
    pub img: RgbaImage,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        // Redirects are followed by hand, so a sign-in cookie never leaves roblox.com.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into()
}

/// Send a request, waiting and retrying when Roblox says "too many requests" (HTTP 429).
fn send(make: impl Fn() -> ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut waited = 0;
    loop {
        let resp = make().call().map_err(|e| format!("could not reach Roblox: {e}"))?;
        if resp.status().as_u16() != 429 || waited >= 20 {
            return Ok(resp);
        }
        let secs = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(3)
            .clamp(1, 8);
        std::thread::sleep(Duration::from_secs(secs));
        waited += secs;
    }
}

fn digits_after(s: &str, marker: &str) -> Option<u64> {
    let i = s.find(marker)? + marker.len();
    let d: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok().filter(|&n| n > 0)
}

/// The asset ID in a catalog / Creator Store link, an `?id=` / `rbxassetid://` link, or a bare number.
pub fn parse_id(text: &str) -> Option<u64> {
    let s = text.trim().to_ascii_lowercase();
    for m in ["/catalog/", "/library/", "/store/asset/", "/marketplace/asset/", "/asset/", "rbxassetid://", "?id=", "&id="] {
        if let Some(n) = digits_after(&s, m) {
            return Some(n);
        }
    }
    if !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok().filter(|&n| n > 0);
    }
    None
}

fn read(resp: &mut ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, String> {
    resp.body_mut()
        .with_config()
        .limit(30 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("download failed: {e}"))
}

fn roblox_error(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["errors"][0]["message"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Download an asset's file. Tries without signing in, then with each saved account.
fn asset_bytes(agent: &ureq::Agent, id: u64, cookies: &[String]) -> Result<Vec<u8>, String> {
    let mut tries: Vec<Option<&str>> = vec![None];
    tries.extend(cookies.iter().map(|c| Some(c.as_str())));
    let mut last = String::new();
    let mut needs_auth = false;
    for cookie in tries {
        // v1 redirects to the CDN. The redirect is followed without the cookie.
        let mut resp = send(|| {
            let req = agent.get(format!("https://assetdelivery.roblox.com/v1/asset/?id={id}"));
            match cookie {
                Some(c) => req.header("Cookie", format!(".ROBLOSECURITY={c}")),
                None => req,
            }
        })?;
        let status = resp.status().as_u16();
        let loc = resp.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
        if let (300..=399, Some(loc)) = (status, &loc) {
            let mut r = send(|| agent.get(loc))?;
            if r.status().as_u16() != 200 {
                return Err(format!("the download failed (HTTP {})", r.status().as_u16()));
            }
            return read(&mut r);
        }
        let body = read(&mut resp)?;
        if status == 200 {
            return Ok(body);
        }
        last = roblox_error(&body);
        if last.is_empty() {
            last = format!("HTTP {status}");
        }
        // Without an account, only an auth error is worth retrying; with one, a problem with that
        // account (moderated, signed out) shouldn't stop the next account being tried.
        if cookie.is_none() {
            needs_auth = status == 401 || status == 403 || last.to_ascii_lowercase().contains("authentication");
            if !needs_auth {
                break;
            }
        } else if !last.to_ascii_lowercase().contains("authentication") && !last.to_ascii_lowercase().contains("moderated") && status != 401 && status != 403 {
            break;
        }
    }
    if needs_auth && cookies.is_empty() {
        return Err("Roblox only gives this item to signed-in users. Sign in to an account on the Accounts tab, then try again.".into());
    }
    if last.to_ascii_lowercase().contains("moderated") {
        return Err("Roblox only gives this item to signed-in users, and the saved account(s) are moderated (banned or warned). Sign in to an account in good standing on the Accounts tab.".into());
    }
    if last == "HTTP 429" {
        return Err("Roblox is limiting downloads right now. Wait a minute, then press Get again.".into());
    }
    Err(format!("Roblox said: {last}"))
}

/// The image ID in a Shirt / Pants / ShirtGraphic model file.
fn template_id(model: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(model);
    let lower = text.to_ascii_lowercase();
    for m in ["?id=", "&amp;id=", "&id=", "rbxassetid://"] {
        if let Some(n) = digits_after(&lower, m) {
            return Some(n);
        }
    }
    None
}

fn details(agent: &ureq::Agent, id: u64) -> Result<(u64, String, String), String> {
    let mut resp = send(|| agent.get(format!("https://economy.roblox.com/v2/assets/{id}/details")))?;
    let status = resp.status().as_u16();
    let body = read(&mut resp)?;
    if status == 404 || status == 400 {
        return Err(format!("there's no Roblox item with ID {id}"));
    }
    if status == 429 {
        return Err("Roblox is limiting lookups right now. Wait a minute, then press Get again.".into());
    }
    if status != 200 {
        return Err(format!("Roblox didn't answer (HTTP {status})"));
    }
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let ty = v["AssetTypeId"].as_u64().unwrap_or(0);
    let name = v["Name"].as_str().unwrap_or("").to_string();
    let creator = v["Creator"]["Name"].as_str().unwrap_or("").to_string();
    Ok((ty, name, creator))
}

pub fn fetch(id: u64, cookies: &[String]) -> Result<Item, String> {
    let agent = agent();
    let (ty, name, creator) = details(&agent, id)?;
    let kind = match ty {
        2 => Some(Kind::TShirt),
        11 => Some(Kind::Shirt),
        12 => Some(Kind::Pants),
        1 | 13 => None, // an image or decal: use it as it is
        _ => {
            let what = match ty {
                8 => "a hat",
                41 => "hair",
                42..=47 => "an accessory",
                64..=69 | 72 => "layered (3D) clothing, which has no 2D template",
                70 | 71 => "a shoe",
                17 | 18 => "a head or face",
                27..=31 => "a body part",
                _ => "not clothing",
            };
            return Err(format!("\"{name}\" is {what}. Paste a classic Shirt, Pants or T-Shirt."));
        }
    };
    let template_id = if kind.is_some() {
        let model = asset_bytes(&agent, id, cookies)?;
        template_id(&model).ok_or("Roblox sent this item in a form the app can't read (no template image in it).")?
    } else {
        id
    };
    let bytes = asset_bytes(&agent, template_id, cookies)?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("the template image couldn't be read: {e}"))?
        .to_rgba8();
    Ok(Item { id, kind, name, creator, template_id, img })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        assert_eq!(parse_id("https://www.roblox.com/catalog/607785314/ROBLOX-Jacket"), Some(607785314));
        assert_eq!(parse_id("https://www.roblox.com/library/123456/x"), Some(123456));
        assert_eq!(parse_id("https://create.roblox.com/store/asset/987654321"), Some(987654321));
        assert_eq!(parse_id("rbxassetid://5555555"), Some(5555555));
        assert_eq!(parse_id("http://www.roblox.com/asset/?id=607785311"), Some(607785311));
        assert_eq!(parse_id("  607785314 "), Some(607785314));
        assert_eq!(parse_id("hello"), None);
        assert_eq!(parse_id(""), None);
    }

    #[test]
    fn finds_template() {
        let xml = br#"<Content name="ShirtTemplate"><url>http://www.roblox.com/asset/?id=607785311</url></Content>"#;
        assert_eq!(template_id(xml), Some(607785311));
        assert_eq!(template_id(b"<url>rbxassetid://42424242</url>"), Some(42424242));
    }

    /// Hits the real Roblox API: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_public_shirt() {
        let item = fetch(607785314, &[]).expect("fetch");
        assert_eq!(item.kind, Some(Kind::Shirt));
        assert_eq!(item.img.dimensions(), (585, 559));
    }
}

#[cfg(test)]
mod live {
    /// Newer items need a saved account: `cargo test live -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_new_items_signed_in() {
        let cookies: Vec<String> = crate::accounts::load().into_iter().map(|a| a.cookie).collect();
        for id in [110046105948243u64, 85625797107085] {
            match super::fetch(id, &cookies) {
                Ok(i) => println!("{id}: {:?} {:?} {}x{}", i.kind, i.name, i.img.width(), i.img.height()),
                Err(e) => panic!("{id}: {e}"),
            }
        }
    }
}


#[cfg(test)]
mod live_kinds {
    #[test]
    #[ignore]
    fn old_pants_and_tshirt() {
        let p = super::fetch(398633812, &[]).expect("pants");
        assert_eq!(p.kind, Some(crate::clothing::Kind::Pants));
        let t = super::fetch(1036727, &[]).expect("tshirt");
        assert_eq!(t.kind, Some(crate::clothing::Kind::TShirt));
        println!("pants {:?} tshirt {:?}", p.img.dimensions(), t.img.dimensions());
    }
}
