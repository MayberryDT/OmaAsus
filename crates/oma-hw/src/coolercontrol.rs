//! CoolerControl daemon client (REST, `http://localhost:11987`).
//!
//! Where CoolerControl runs and is authorised, OmaAsus delegates fan and pump
//! control to it (its profiles and modes), so one owner drives each PWM
//! output.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcChannel {
    pub name: String,
    pub label: Option<String>,
    pub min_duty: Option<u8>,
    pub max_duty: Option<u8>,
    pub fixed_enabled: bool,
    pub lighting: bool,
    pub lcd: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcDevice {
    pub uid: String,
    pub name: String,
    pub d_type: String,
    pub channels: Vec<CcChannel>,
    pub temps: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcProfile {
    pub uid: String,
    pub name: String,
    pub p_type: String,
    pub speed_fixed: Option<u8>,
    pub speed_profile: Vec<(f64, u8)>,
    pub temp_source: Option<(String, String)>,
    pub function_uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcMode {
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcChannelStatus {
    pub rpm: Option<f64>,
    pub duty: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcStatus {
    /// device uid → channel name → status
    pub channels: BTreeMap<String, BTreeMap<String, CcChannelStatus>>,
    /// device uid → temp name → °C
    pub temps: BTreeMap<String, BTreeMap<String, f64>>,
}

#[derive(Clone)]
pub struct CoolerControl {
    base: String,
    http: reqwest::Client,
    password: Option<Arc<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CcError {
    #[error("CoolerControl: {0}")]
    Http(#[from] reqwest::Error),
    #[error("CoolerControl: {0}")]
    Api(String),
    #[error("CoolerControl: not authorised (check the password in Settings)")]
    Unauthorised,
}

impl CoolerControl {
    pub fn new(base: &str, password: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(5))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest client");
        Self { base: base.trim_end_matches('/').to_string(), http, password: password.map(Arc::new) }
    }

    pub async fn handshake(&self) -> bool {
        self.http.get(format!("{}/handshake", self.base)).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    pub async fn login(&self) -> Result<(), CcError> {
        let pw = self.password.as_deref().map(String::as_str).unwrap_or("coolAdmin");
        let r = self.http.post(format!("{}/login", self.base)).basic_auth("CCAdmin", Some(pw)).send().await?;
        if r.status().is_success() { Ok(()) } else { Err(CcError::Unauthorised) }
    }

    async fn check(r: reqwest::Response) -> Result<Value, CcError> {
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if status == reqwest::StatusCode::UNAUTHORIZED || v.get("error").and_then(Value::as_str).is_some_and(|e| e.contains("Credentials")) {
            return Err(CcError::Unauthorised);
        }
        if !status.is_success() {
            return Err(CcError::Api(v.get("error").and_then(Value::as_str).unwrap_or("request failed").to_string()));
        }
        Ok(v)
    }

    async fn get(&self, path: &str) -> Result<Value, CcError> {
        let r = self.http.get(format!("{}{}", self.base, path)).send().await?;
        match Self::check(r).await {
            Err(CcError::Unauthorised) => {
                self.login().await?;
                Self::check(self.http.get(format!("{}{}", self.base, path)).send().await?).await
            }
            other => other,
        }
    }

    async fn send_json(&self, method: reqwest::Method, path: &str, body: &Value) -> Result<Value, CcError> {
        let mk = || self.http.request(method.clone(), format!("{}{}", self.base, path)).json(body);
        match Self::check(mk().send().await?).await {
            Err(CcError::Unauthorised) => {
                self.login().await?;
                Self::check(mk().send().await?).await
            }
            other => other,
        }
    }

    pub async fn devices(&self) -> Result<Vec<CcDevice>, CcError> {
        let v = self.get("/devices").await?;
        let arr = v.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .map(|d| {
                let info = &d["info"];
                let channels = info["channels"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(name, c)| {
                                let so = &c["speed_options"];
                                CcChannel {
                                    name: name.clone(),
                                    label: c["label"].as_str().map(str::to_string),
                                    min_duty: so["min_duty"].as_u64().map(|x| x as u8),
                                    max_duty: so["max_duty"].as_u64().map(|x| x as u8),
                                    fixed_enabled: so["fixed_enabled"].as_bool().unwrap_or(false),
                                    lighting: c["lighting_modes"].as_array().is_some_and(|a| !a.is_empty()),
                                    lcd: c["lcd_modes"].as_array().is_some_and(|a| !a.is_empty()),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let temps = info["temps"]
                    .as_object()
                    .map(|m| m.iter().map(|(n, t)| (n.clone(), t["label"].as_str().unwrap_or(n).to_string())).collect())
                    .unwrap_or_default();
                CcDevice { uid: d["uid"].as_str().unwrap_or("").into(), name: d["name"].as_str().unwrap_or("").into(), d_type: d["type"].as_str().or(d["d_type"].as_str()).unwrap_or("").into(), channels, temps }
            })
            .collect())
    }

    pub async fn status(&self) -> Result<CcStatus, CcError> {
        let v = self.send_json(reqwest::Method::POST, "/status", &json!({})).await?;
        let mut out = CcStatus::default();
        for d in v["devices"].as_array().cloned().unwrap_or_default() {
            let uid = d["uid"].as_str().unwrap_or("").to_string();
            if let Some(last) = d["status_history"].as_array().and_then(|h| h.last()) {
                let ch = out.channels.entry(uid.clone()).or_default();
                for c in last["channels"].as_array().cloned().unwrap_or_default() {
                    ch.insert(c["name"].as_str().unwrap_or("").into(), CcChannelStatus { rpm: c["rpm"].as_f64(), duty: c["duty"].as_f64() });
                }
                let t = out.temps.entry(uid).or_default();
                for x in last["temps"].as_array().cloned().unwrap_or_default() {
                    if let Some(v) = x["temp"].as_f64() {
                        t.insert(x["name"].as_str().unwrap_or("").into(), v);
                    }
                }
            }
        }
        Ok(out)
    }

    pub async fn profiles(&self) -> Result<Vec<CcProfile>, CcError> {
        let v = self.get("/profiles").await?;
        Ok(v["profiles"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|p| CcProfile {
                uid: p["uid"].as_str().unwrap_or("").into(),
                name: p["name"].as_str().unwrap_or("").into(),
                p_type: p["p_type"].as_str().unwrap_or("").into(),
                speed_fixed: p["speed_fixed"].as_u64().map(|x| x as u8),
                speed_profile: p["speed_profile"].as_array().map(|a| a.iter().filter_map(|pt| Some((pt[0].as_f64()?, pt[1].as_u64()? as u8))).collect()).unwrap_or_default(),
                temp_source: p["temp_source"].as_object().map(|t| (t["device_uid"].as_str().unwrap_or("").into(), t["temp_name"].as_str().unwrap_or("").into())),
                function_uid: p["function_uid"].as_str().unwrap_or("0").into(),
            })
            .collect())
    }

    /// Create or update a graph profile owned by OmaAsus. Returns its uid.
    pub async fn upsert_graph_profile(&self, uid: Option<&str>, name: &str, points: &[(f64, u8)], temp_device: &str, temp_name: &str, function_uid: &str) -> Result<String, CcError> {
        let uid = uid.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let body = json!({
            "uid": uid, "name": name, "p_type": "Graph",
            "speed_profile": points.iter().map(|(t, d)| json!([t, d])).collect::<Vec<_>>(),
            "temp_source": {"device_uid": temp_device, "temp_name": temp_name},
            "function_uid": function_uid,
        });
        let existing = self.profiles().await?.iter().any(|p| p.uid == uid);
        let method = if existing { reqwest::Method::PUT } else { reqwest::Method::POST };
        self.send_json(method, "/profiles", &body).await?;
        Ok(uid)
    }

    pub async fn set_manual(&self, device_uid: &str, channel: &str, duty: u8) -> Result<(), CcError> {
        self.send_json(reqwest::Method::PUT, &format!("/devices/{device_uid}/settings/{channel}/manual"), &json!({"speed_fixed": duty})).await.map(|_| ())
    }

    pub async fn set_profile(&self, device_uid: &str, channel: &str, profile_uid: &str) -> Result<(), CcError> {
        self.send_json(reqwest::Method::PUT, &format!("/devices/{device_uid}/settings/{channel}/profile"), &json!({"profile_uid": profile_uid})).await.map(|_| ())
    }

    pub async fn reset(&self, device_uid: &str, channel: &str) -> Result<(), CcError> {
        self.send_json(reqwest::Method::PUT, &format!("/devices/{device_uid}/settings/{channel}/reset"), &json!({})).await.map(|_| ())
    }

    pub async fn modes(&self) -> Result<Vec<CcMode>, CcError> {
        let v = self.get("/modes").await?;
        Ok(v["modes"].as_array().cloned().unwrap_or_default().iter().map(|m| CcMode { uid: m["uid"].as_str().unwrap_or("").into(), name: m["name"].as_str().unwrap_or("").into() }).collect())
    }

    pub async fn active_mode(&self) -> Result<Option<String>, CcError> {
        let v = self.get("/modes-active").await?;
        Ok(v["current_mode_uids"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string).or_else(|| v.as_array().and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string)))
    }

    pub async fn activate_mode(&self, uid: &str) -> Result<(), CcError> {
        self.send_json(reqwest::Method::POST, &format!("/modes-active/{uid}"), &json!({})).await.map(|_| ())
    }

    /// Snapshot current device settings as a new Mode with this name.
    pub async fn save_mode(&self, name: &str) -> Result<String, CcError> {
        let v = self.send_json(reqwest::Method::POST, "/modes", &json!({"name": name})).await?;
        Ok(v["uid"].as_str().unwrap_or("").to_string())
    }
}
