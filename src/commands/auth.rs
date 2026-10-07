//! Browser sign-in with device approval.

use std::time::{Duration, Instant};

use anyhow::bail;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::print;
use crate::{
    api::{Api, decode},
    config::Settings,
};

#[derive(Deserialize)]
struct DeviceLogin {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct Session {
    token: String,
}

pub fn login(api: &Api, no_open: bool) -> anyhow::Result<()> {
    let login: DeviceLogin = decode(api.anonymous(Method::POST, "/v1/auth/device").send()?)?;
    eprintln!("Confirm this code in your browser: {}", login.user_code);
    if no_open || webbrowser::open(&login.verification_url).is_err() {
        eprintln!("Open {}", login.verification_url);
    }
    let token = wait_for_approval(api, &login)?;
    let account: Value = api.with_token(token.clone()).get("/v1/account")?;
    Settings {
        endpoint: Some(api.endpoint().into()),
        token: Some(token),
    }
    .save()?;
    print(&json!({ "status": "signed_in", "account": account }))
}

pub fn logout(api: &Api) -> anyhow::Result<()> {
    Settings {
        endpoint: Some(api.endpoint().into()),
        token: None,
    }
    .save()?;
    print(&json!({ "status": "signed_out" }))
}

fn wait_for_approval(api: &Api, login: &DeviceLogin) -> anyhow::Result<String> {
    let deadline = Instant::now() + Duration::from_secs(login.expires_in);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(login.interval.max(1)));
        let response = api
            .anonymous(Method::POST, "/v1/auth/device/token")
            .json(&json!({ "device_code": login.device_code }))
            .send()?;
        if response.status() != StatusCode::ACCEPTED {
            return Ok(decode::<Session>(response)?.token);
        }
    }
    bail!("sign-in expired; run `ssp login` again")
}
