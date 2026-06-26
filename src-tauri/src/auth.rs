use keyring_core::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::error;

use crate::auth;

const DEFAULT_AUTH_URL: &str = "http://localhost:3000";
const FORCE_AUTH_URL_ENV: &str = "FORCE_AUTH_URL";

#[derive(Debug)]
pub enum AuthError {
    Network(String),
    RateLimited,
    Unauthorized,
    UpgradeRequired,
    Client(u16),
    Server(u16),
    InvalidResponse(String),
}

#[derive(Serialize)]
pub struct LoginError {
    code: String,
    message: String,
}

impl LoginError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

fn map_auth_status(status: StatusCode) -> AuthError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AuthError::RateLimited;
    }

    if status == StatusCode::UNAUTHORIZED {
        return AuthError::Unauthorized;
    }

    if status == StatusCode::UPGRADE_REQUIRED {
        return AuthError::UpgradeRequired;
    }

    if status.is_client_error() {
        return AuthError::Client(status.as_u16());
    }

    AuthError::Server(status.as_u16())
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

    if !response.status().is_success() {
        error!(response = %response.status(), "Auth failed");
        return Err(map_auth_status(response.status()));
    }

    response
        .json::<LoginResponse>()
        .await
        .map_err(|e| AuthError::InvalidResponse(e.to_string()))
}

pub async fn extend_session(token: &String) -> Result<bool, AuthError> {
    let client = Client::new();

    for attempt in 1..=3 {
        let response = client
            .post(format!("{}/extend", auth_url()))
            .json(&ExtendRequest {
                token: token.to_string(),
                version: crate::VERSION.to_string(),
            })
            .send()
            .await;

        match response {
            Ok(response) => {
                if response.status().is_success() {
                    return Ok(true);
                }

                if response.status().is_client_error() {
                    return Ok(false);
                }

                return Err(map_auth_status(response.status()));
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
        .map_err(|err| match err {
            AuthError::RateLimited => LoginError::new(
                "RATE_LIMITED",
                "Too many requests. Please wait and try again.",
            ),
            AuthError::Unauthorized => {
                LoginError::new("INVALID_CREDENTIALS", "Invalid username or password.")
            }
            AuthError::UpgradeRequired => LoginError::new(
                "UPGRADE_REQUIRED",
                "Launcher is outdated. Please update to the latest version.",
            ),
            AuthError::Client(status) => {
                LoginError::new("AUTH_FAILED", format!("Auth failed: {}", status))
            }
            AuthError::Server(status) => {
                LoginError::new("SERVER_ERROR", format!("Server error: {}", status))
            }
            AuthError::Network(message) => LoginError::new("NETWORK_ERROR", message),
            AuthError::InvalidResponse(message) => LoginError::new("BAD_RESPONSE", message),
        })?;

    let token_entry = Entry::new("saykocraft-launcher", "session").map_err(|e| LoginError {
        code: "KEYRING_ACCESS".to_string(),
        message: format!("Couldn't access keyring: {}", e),
    })?;

    token_entry
        .set_password(&response.token)
        .map_err(|e| LoginError {
            code: "KEYRING_SAVE".to_string(),
            message: format!("Couldn't save token: {}", e),
        })?;

    let username_entry = Entry::new("saykocraft-launcher", "username").map_err(|e| LoginError {
        code: "KEYRING_ACCESS".to_string(),
        message: format!("Couldn't access keyring: {}", e),
    })?;

    username_entry
        .set_password(&response.username)
        .map_err(|e| LoginError {
            code: "KEYRING_SAVE".to_string(),
            message: format!("Couldn't save username: {}", e),
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
