use keyring_core::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;
use tracing::error;

use crate::auth;

const DEFAULT_AUTH_URL: &str = "https://nf.blacksmith-ent.com/auth";
const FORCE_AUTH_URL_ENV: &str = "FORCE_AUTH_URL";

#[derive(Debug)]
pub enum AuthError {
    Network(String),
    RateLimited,
    Unauthorized,
    AccountNotApproved,
    Banned(Option<i64>),
    UpgradeRequired,
    Maintenance,
    Client(u16),
    Server(u16),
    InvalidResponse(String),
}

#[derive(Clone, Serialize)]
pub struct LoginError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
}

impl LoginError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            expires_at: None,
        }
    }

    fn banned(expires_at: Option<i64>) -> Self {
        Self {
            code: "BANNED".to_string(),
            message: "This account is banned.".to_string(),
            expires_at,
        }
    }
}

pub fn login_error_from_auth_error(err: AuthError) -> LoginError {
    match err {
        AuthError::RateLimited => LoginError::new(
            "RATE_LIMITED",
            "Too many requests. Please wait and try again.",
        ),
        AuthError::Unauthorized => {
            LoginError::new("INVALID_CREDENTIALS", "Invalid username or password.")
        }
        AuthError::AccountNotApproved => LoginError::new(
            "ACCOUNT_NOT_APPROVED",
            "Your account has not been approved yet!",
        ),
        AuthError::Banned(expires_at) => LoginError::banned(expires_at),
        AuthError::UpgradeRequired => LoginError::new(
            "UPGRADE_REQUIRED",
            "Launcher is outdated. Please update to the latest version.",
        ),
        AuthError::Maintenance => LoginError::new(
            "SERVICE_UNAVAILABLE",
            "Auth service is temporarily unavailable for maintenance.",
        ),
        AuthError::Client(status) => {
            LoginError::new("AUTH_FAILED", format!("Auth failed: {}", status))
        }
        AuthError::Server(status) => {
            LoginError::new("SERVER_ERROR", format!("Server error: {}", status))
        }
        AuthError::Network(message) => LoginError::new("NETWORK_ERROR", message),
        AuthError::InvalidResponse(message) => LoginError::new("BAD_RESPONSE", message),
    }
}

fn map_auth_status(status: StatusCode) -> AuthError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AuthError::RateLimited;
    }

    if status == StatusCode::UNAUTHORIZED {
        return AuthError::Unauthorized;
    }

    if status == StatusCode::FORBIDDEN {
        return AuthError::Banned(None);
    }

    if status == StatusCode::UPGRADE_REQUIRED {
        return AuthError::UpgradeRequired;
    }

    if status == StatusCode::SERVICE_UNAVAILABLE {
        return AuthError::Maintenance;
    }

    if status.is_client_error() {
        return AuthError::Client(status.as_u16());
    }

    AuthError::Server(status.as_u16())
}

fn f64_to_i64(value: f64) -> Option<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return None;
    }

    Some(value as i64)
}

fn parse_epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| number.as_f64().and_then(f64_to_i64)),
        Value::String(value) => value
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| value.trim().parse::<f64>().ok().and_then(f64_to_i64)),
        _ => None,
    }
}

async fn map_auth_response(response: reqwest::Response) -> AuthError {
    let status = response.status();

    if status == StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        let parsed_body = serde_json::from_str::<Value>(&body).ok();
        let message = parsed_body
            .as_ref()
            .and_then(|value| {
                value.as_str().or_else(|| {
                    value.as_object().and_then(|object| {
                        ["message", "error", "detail"]
                            .iter()
                            .find_map(|key| object.get(*key).and_then(Value::as_str))
                    })
                })
            })
            .unwrap_or(body.trim());

        if message.trim() == "Account not yet created" {
            return AuthError::AccountNotApproved;
        }

        let expires_at = parsed_body.as_ref().and_then(|value| {
            value.as_object().and_then(|object| {
                object
                    .get("expires_at")
                    .or_else(|| object.get("expiresAt"))
                    .and_then(parse_epoch)
            })
        });

        return AuthError::Banned(expires_at);
    }

    map_auth_status(status)
}

fn is_network_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout()
}

fn auth_url() -> String {
    std::env::var(FORCE_AUTH_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_AUTH_URL.to_string())
}

#[derive(Debug, Serialize)]
struct LoginRequest {
    username: String,
    password: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct ExtendRequest {
    token: String,
    username: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct LogoutRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub username: String,
    pub token: String,
}

pub async fn authenticate(username: &str, password: &str) -> Result<LoginResponse, AuthError> {
    let client = Client::new();

    let response = client
        .post(format!("{}/login", auth_url()))
        .json(&LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
            version: crate::VERSION.to_string(),
        })
        .send()
        .await
        .map_err(|e| AuthError::Network(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        error!(response = %status, "Auth failed");
        return Err(map_auth_response(response).await);
    }

    response
        .json::<LoginResponse>()
        .await
        .map_err(|e| AuthError::InvalidResponse(e.to_string()))
}

pub async fn extend_session(token: &String, username: &String) -> Result<bool, AuthError> {
    let client = Client::new();

    for attempt in 1..=3 {
        let response = client
            .post(format!("{}/extend", auth_url()))
            .json(&ExtendRequest {
                token: token.to_string(),
                username: username.to_string(),
                version: crate::VERSION.to_string(),
            })
            .send()
            .await;

        match response {
            Ok(response) => {
                let status = response.status();

                if status.is_success() {
                    return Ok(true);
                }

                if status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND {
                    return Ok(false);
                }

                let error = map_auth_response(response).await;
                return Err(error);
            }
            Err(e) => {
                if !is_network_error(&e) {
                    return Err(AuthError::InvalidResponse(e.to_string()));
                }

                if attempt == 3 {
                    return Err(AuthError::Network(e.to_string()));
                }

                sleep(Duration::from_millis(1000)).await;
            }
        }
    }

    Err(AuthError::Network(
        "Network error: request retries exhausted".to_string(),
    ))
}

pub async fn logout_session(token: &String) -> StatusCode {
    let client = Client::new();

    match client
        .post(format!("{}/logout", auth_url()))
        .json(&LogoutRequest {
            token: token.to_string(),
        })
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                error!(response = %response.status(), "Logout failed but result is ignored");
            }

            response.status()
        }
        Err(e) => {
            error!(err = %e, "Logout failed but result is ignored");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[tauri::command]
pub async fn login(username: String, password: String) -> Result<String, LoginError> {
    let response = auth::authenticate(&username, &password)
        .await
        .map_err(login_error_from_auth_error)?;

    let token_entry = Entry::new("saykocraft-launcher", "session").map_err(|e| LoginError {
        code: "KEYRING_ACCESS".to_string(),
        message: format!("Couldn't access keyring: {}", e),
        expires_at: None,
    })?;

    token_entry
        .set_password(&response.token)
        .map_err(|e| LoginError {
            code: "KEYRING_SAVE".to_string(),
            message: format!("Couldn't save token: {}", e),
            expires_at: None,
        })?;

    let username_entry = Entry::new("saykocraft-launcher", "username").map_err(|e| LoginError {
        code: "KEYRING_ACCESS".to_string(),
        message: format!("Couldn't access keyring: {}", e),
        expires_at: None,
    })?;

    username_entry
        .set_password(&response.username)
        .map_err(|e| LoginError {
            code: "KEYRING_SAVE".to_string(),
            message: format!("Couldn't save username: {}", e),
            expires_at: None,
        })?;

    Ok("Login successful".to_string())
}

#[tauri::command]
pub async fn get_session_token() -> Result<Option<String>, String> {
    let token_entry = Entry::new("saykocraft-launcher", "session")
        .map_err(|e| format!("Couldn't access keyring: {}", e))?;

    match token_entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(format!("Failed to read token: {}", e)),
    }
}
