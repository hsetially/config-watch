use web_sys::HtmlInputElement;
use yew::{function_component, html, Callback, Html, Properties, TargetCast, UseStateHandle};

use crate::auth;

#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    SignIn,
    SignUp,
}

#[derive(Debug, Clone, PartialEq, Properties)]
pub struct LoginProps {
    pub on_success: Callback<crate::storage::AuthData>,
    pub on_approval_pending: Callback<()>,
}

#[function_component(Login)]
pub fn login(props: &LoginProps) -> Html {
    let mode: UseStateHandle<AuthMode> = yew::use_state(|| AuthMode::SignIn);
    let email: UseStateHandle<String> = yew::use_state(String::new);
    let password: UseStateHandle<String> = yew::use_state(String::new);
    let confirm_password: UseStateHandle<String> = yew::use_state(String::new);
    let name: UseStateHandle<String> = yew::use_state(String::new);
    let error: UseStateHandle<Option<String>> = yew::use_state(|| None);
    let loading: UseStateHandle<bool> = yew::use_state(|| false);

    let on_email_change = {
        let email = email.clone();
        Callback::from(move |e: yew::Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            email.set(input.value());
        })
    };

    let on_password_change = {
        let password = password.clone();
        Callback::from(move |e: yew::Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            password.set(input.value());
        })
    };

    let on_confirm_password_change = {
        let confirm_password = confirm_password.clone();
        Callback::from(move |e: yew::Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            confirm_password.set(input.value());
        })
    };

    let on_name_change = {
        let name = name.clone();
        Callback::from(move |e: yew::Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            name.set(input.value());
        })
    };

    let on_submit = {
        let mode = mode.clone();
        let email = email.clone();
        let password = password.clone();
        let confirm_password = confirm_password.clone();
        let name = name.clone();
        let error = error.clone();
        let loading = loading.clone();
        let on_success = props.on_success.clone();
        let on_approval_pending = props.on_approval_pending.clone();

        Callback::from(move |e: web_sys::SubmitEvent| {
            e.prevent_default();
            error.set(None);

            let email_val = (*email).clone();
            let password_val = (*password).clone();

            if email_val.is_empty() || password_val.is_empty() {
                error.set(Some("Email and password are required".to_string()));
                return;
            }

            if *mode == AuthMode::SignUp {
                let confirm = (*confirm_password).clone();
                if password_val != confirm {
                    error.set(Some("Passwords do not match".to_string()));
                    return;
                }
            }

            loading.set(true);

            let base_url = "localhost:8082";

            let loading_err = loading.clone();
            let error_err = error.clone();
            let on_success_clone = on_success.clone();
            let on_approval_pending_clone = on_approval_pending.clone();
            match *mode {
                AuthMode::SignIn => {
                    let loading_err2 = loading_err.clone();
                    let _error_err2 = error_err.clone();
                    let on_approval_pending2 = on_approval_pending_clone.clone();
                    auth::sign_in(
                        base_url,
                        &email_val,
                        &password_val,
                        on_success_clone,
                        Callback::from(move |()| {
                            loading_err2.set(false);
                            on_approval_pending2.emit(());
                        }),
                        Callback::from(move |err| {
                            loading_err.set(false);
                            error_err.set(Some(err));
                        }),
                    );
                }
                AuthMode::SignUp => {
                    let name_val = (*name).clone();
                    let name_opt = if name_val.is_empty() { None } else { Some(name_val.as_str()) };
                    let loading_err2 = loading_err.clone();
                    let error_err2 = error_err.clone();
                    let on_approval_pending2 = on_approval_pending_clone.clone();
                    auth::sign_up(
                        base_url,
                        &email_val,
                        &password_val,
                        name_opt,
                        on_success_clone,
                        Callback::from(move |()| {
                            loading_err2.set(false);
                            on_approval_pending2.emit(());
                        }),
                        Callback::from(move |err| {
                            error_err2.set(Some(err));
                        }),
                    );
                }
            }
        })
    };

    let toggle_mode = {
        let mode = mode.clone();
        let error = error.clone();
        Callback::from(move |_| {
            match *mode {
                AuthMode::SignIn => mode.set(AuthMode::SignUp),
                AuthMode::SignUp => mode.set(AuthMode::SignIn),
            }
            error.set(None);
        })
    };

    let is_sign_up = *mode == AuthMode::SignUp;
    let is_loading = *loading;
    let title = if is_sign_up { "Create Account" } else { "Sign In" };
    let toggle_text = if is_sign_up {
        "Already have an account? Sign in"
    } else {
        "Don't have an account? Sign up"
    };

    html! {
        <div class="login-container">
            <div class="login-card">
                <h1 class="login-title">{ "Config Watch" }</h1>
                <h2 class="login-subtitle">{ title }</h2>

                if let Some(err) = &*error {
                    <div class="login-error">{ err }</div>
                }

                <form onsubmit={on_submit}>
                    if is_sign_up {
                        <div class="login-field">
                            <label for="name">{ "Name (optional)" }</label>
                            <input
                                id="name"
                                type="text"
                                placeholder="Your name"
                                value={(*name).clone()}
                                onchange={on_name_change}
                                disabled={is_loading}
                            />
                        </div>
                    }

                    <div class="login-field">
                        <label for="email">{ "Email" }</label>
                        <input
                            id="email"
                            type="email"
                            placeholder="you@example.com"
                            value={(*email).clone()}
                            onchange={on_email_change}
                            disabled={is_loading}
                            required=true
                        />
                    </div>

                    <div class="login-field">
                        <label for="password">{ "Password" }</label>
                        <input
                            id="password"
                            type="password"
                            placeholder="Password"
                            value={(*password).clone()}
                            onchange={on_password_change}
                            disabled={is_loading}
                            required=true
                        />
                    </div>

                    if is_sign_up {
                        <div class="login-field">
                            <label for="confirm-password">{ "Confirm Password" }</label>
                            <input
                                id="confirm-password"
                                type="password"
                                placeholder="Confirm password"
                                value={(*confirm_password).clone()}
                                onchange={on_confirm_password_change}
                                disabled={is_loading}
                                required=true
                            />
                        </div>
                    }

                    <button type="submit" class="login-submit" disabled={is_loading}>
                        if is_loading {
                            { "Signing in..." }
                        } else if is_sign_up {
                            { "Create Account" }
                        } else {
                            { "Sign In" }
                        }
                    </button>
                </form>

                <button class="login-toggle" onclick={toggle_mode} disabled={is_loading}>
                    { toggle_text }
                </button>
            </div>
        </div>
    }
}