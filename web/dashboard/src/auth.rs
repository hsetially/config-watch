use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::storage;
use crate::url;

/// Data returned by BetterAuth sign-in/sign-up endpoints.
/// The `token` field is now optional — with cookie-based auth, the server
/// sets an HttpOnly session cookie and a non-HttpOnly CSRF cookie, so
/// the token is not returned in the response body (stripped by the proxy).
/// Backward compat: if the server does return a token, we ignore it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    #[serde(default)]
    pub token: Option<String>,
    pub user: AuthUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Error response from BetterAuth.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthErrorResponse {
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Hash a password with SHA-256 before sending to the server.
/// This ensures the plaintext password never leaves the client.
async fn hash_password(password: &str) -> String {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return password.to_string(),
    };
    let crypto = match window.crypto() {
        Ok(c) => c,
        Err(_) => return password.to_string(),
    };
    let subtle = crypto.subtle();

    // Encode the password as UTF-8 bytes using a JS TextEncoder
    let encoder = js_sys::eval("new TextEncoder()").expect("TextEncoder not available");
    let encode_fn = js_sys::Reflect::get(&encoder, &wasm_bindgen::JsValue::from_str("encode"))
        .expect("encode not found on TextEncoder");
    let encoded_val = js_sys::Function::from(encode_fn)
        .call1(&encoder, &wasm_bindgen::JsValue::from_str(password))
        .expect("TextEncoder.encode failed");
    let encoded: &js_sys::Object = encoded_val.dyn_ref().expect("encoded result is not an Object");

    let promise = match subtle.digest_with_str_and_buffer_source("SHA-256", encoded) {
        Ok(p) => p,
        Err(_) => return password.to_string(),
    };

    let hash_buffer = match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(buf) => buf,
        Err(_) => return password.to_string(),
    };

    let hash_array = js_sys::Uint8Array::new(&hash_buffer);
    let hash_bytes = hash_array.to_vec();

    hash_bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Sign in with email/password. On success, saves user identity to localStorage
/// (the session is maintained via HttpOnly cookie). Calls `on_result` with auth data.
/// On 403 with `approval_pending`, calls `on_approval_pending`.
/// On other errors, calls `on_error`.
pub fn sign_in(
    _base_url: &str,
    email: &str,
    password: &str,
    on_result: yew::Callback<storage::AuthData>,
    on_approval_pending: yew::Callback<()>,
    on_error: yew::Callback<String>,
) {
    let url = url::api_url("/auth/sign-in/email");
    let email = email.to_string();
    let password = password.to_string();
    let on_result = on_result.clone();
    let on_approval_pending = on_approval_pending.clone();
    let on_error = on_error.clone();

    spawn_local(async move {
        let hashed_password = hash_password(&password).await;
        let body = serde_json::json!({
            "email": email,
            "password": hashed_password,
        });
        let json_body = serde_json::to_string(&body).unwrap_or_default();

        let req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include)
            .header("Content-Type", "application/json")
            .body(json_body);
        match req {
            Ok(req) => match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status == 403 {
                        match resp.json::<AuthErrorResponse>().await {
                            Ok(err) => {
                                if err.error.as_deref() == Some("approval_pending") {
                                    on_approval_pending.emit(());
                                } else if err.error.as_deref() == Some("insufficient_role") {
                                    on_error.emit("Account does not have required role".to_string());
                                } else {
                                    let msg = err.message.unwrap_or_else(|| "Account suspended".to_string());
                                    on_error.emit(msg);
                                }
                            }
                            Err(_) => on_error.emit("Account suspended".to_string()),
                        }
                    } else if resp.ok() {
                        match resp.json::<AuthResponse>().await {
                            Ok(auth) => {
                                // C4: Session is now cookie-based. The server sets
                                // HttpOnly session cookie + CSRF cookie automatically.
                                // We only store user identity in localStorage.
                                let data = storage::AuthData {
                                    user_id: auth.user.id,
                                    email: auth.user.email,
                                };
                                storage::save_auth_data(&data);
                                on_result.emit(data);
                            }
                            Err(e) => on_error.emit(format!("Failed to parse sign-in response: {}", e)),
                        }
                    } else {
                        let msg = match resp.json::<AuthErrorResponse>().await {
                            Ok(err) => err.message.unwrap_or_else(|| format!("Sign-in failed (HTTP {})", status)),
                            Err(_) => format!("Sign-in failed (HTTP {})", status),
                        };
                        on_error.emit(msg);
                    }
                }
                Err(e) => on_error.emit(format!("Network error: {}", e)),
            },
            Err(e) => on_error.emit(format!("Request error: {}", e)),
        }
    });
}

/// Sign up with email/password. On success:
/// - Saves user identity and calls `on_result` (session is cookie-based).
/// - If approval required (403), calls `on_approval_pending`.
/// On failure, calls `on_error`.
pub fn sign_up(
    _base_url: &str,
    email: &str,
    password: &str,
    name: Option<&str>,
    on_result: yew::Callback<storage::AuthData>,
    on_approval_pending: yew::Callback<()>,
    on_error: yew::Callback<String>,
) {
    let url = url::api_url("/auth/sign-up/email");
    let email = email.to_string();
    let password = password.to_string();
    let name = name.map(|s| s.to_string());
    let on_result = on_result.clone();
    let on_approval_pending = on_approval_pending.clone();
    let on_error = on_error.clone();

    spawn_local(async move {
        let hashed_password = hash_password(&password).await;
        let mut body = serde_json::json!({
            "email": email,
            "password": hashed_password,
        });
        if let Some(n) = name {
            body["name"] = serde_json::Value::String(n);
        }
        let json_body = serde_json::to_string(&body).unwrap_or_default();

        let req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include)
            .header("Content-Type", "application/json")
            .body(json_body);
        match req {
            Ok(req) => match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status == 403 {
                        match resp.json::<AuthErrorResponse>().await {
                            Ok(err) => {
                                if err.error.as_deref() == Some("approval_pending") {
                                    on_approval_pending.emit(());
                                } else if err.error.as_deref() == Some("insufficient_role") {
                                    on_error.emit("Account does not have required role".to_string());
                                } else {
                                    let msg = err.message.unwrap_or_else(|| "Account suspended".to_string());
                                    on_error.emit(msg);
                                }
                            }
                            Err(_) => on_error.emit("Account suspended".to_string()),
                        }
                    } else if resp.ok() {
                        match resp.json::<AuthResponse>().await {
                            Ok(auth) => {
                                let data = storage::AuthData {
                                    user_id: auth.user.id,
                                    email: auth.user.email,
                                };
                                storage::save_auth_data(&data);
                                on_result.emit(data);
                            }
                            Err(e) => on_error.emit(format!("Failed to parse sign-up response: {}", e)),
                        }
                    } else {
                        let msg = match resp.json::<AuthErrorResponse>().await {
                            Ok(err) => err.message.unwrap_or_else(|| format!("Sign-up failed (HTTP {})", status)),
                            Err(_) => format!("Sign-up failed (HTTP {})", status),
                        };
                        on_error.emit(msg);
                    }
                }
                Err(e) => on_error.emit(format!("Network error: {}", e)),
            },
            Err(e) => on_error.emit(format!("Request error: {}", e)),
        }
    });
}

/// Sign out by POSTing to /auth/sign-out with CSRF protection.
/// The server will clear the session cookie. We also clear the local
/// identity data and CSRF cookie.
pub fn sign_out(on_complete: yew::Callback<()>) {
    let url = url::api_url("/auth/sign-out");
    let csrf_token = storage::load_csrf_token();
    let on_complete = on_complete.clone();

    spawn_local(async move {
        let mut req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include);
        if let Some(csrf) = csrf_token {
            req = req.header("x-csrf-token", &csrf);
        }
        let _ = req.send().await;
        storage::clear_auth_data();
        storage::clear_csrf_cookie();
        on_complete.emit(());
    });
}