use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::services::AppState;

pub fn apply_middleware(router: Router, state: &AppState) -> Router {
    let allowed_origins: Vec<String> = state.auth.config().trusted_origins.clone();
    let origins: Vec<_> = allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([
            CONTENT_TYPE,
            AUTHORIZATION,
            HeaderName::from_static("x-agent-token"),
            HeaderName::from_static("x-enrollment-token"),
            HeaderName::from_static("x-idempotency-key"),
            HeaderName::from_static("x-api-key"),
            HeaderName::from_static("x-csrf-token"),
            HeaderName::from_static("x-admin-secret"),
        ])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(3600));

    // H4: Security response headers
    let hsts = SetResponseHeaderLayer::if_not_present(
        axum::http::header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );
    let nosniff = SetResponseHeaderLayer::if_not_present(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let frame_options = SetResponseHeaderLayer::if_not_present(
        axum::http::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    let referrer_policy = SetResponseHeaderLayer::if_not_present(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let csp = SetResponseHeaderLayer::if_not_present(
        axum::http::header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none'; object-src 'none';",
        ),
    );

    router
        .layer(cors)
        .layer(hsts)
        .layer(nosniff)
        .layer(frame_options)
        .layer(referrer_policy)
        .layer(csp)
        .layer(RequestBodyLimitLayer::new(8 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
}