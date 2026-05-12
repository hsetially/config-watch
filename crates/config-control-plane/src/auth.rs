use std::sync::Arc;

use better_auth::{
    adapters::SqlxAdapter,
    core::auth::{AuthBuilder, BetterAuth},
    plugins::{
        admin::AdminPlugin, api_key::ApiKeyConfig, api_key::ApiKeyPlugin,
        email_password::EmailPasswordPlugin, organization::OrganizationPlugin,
        session_management::SessionManagementPlugin, two_factor::TwoFactorPlugin,
    },
    AuthConfig as BetterAuthConfig, SameSite,
};

use crate::config::AuthConfig as AppConfig;

/// Type alias for the auth state used throughout the application.
pub type AuthState = Arc<BetterAuth<SqlxAdapter>>;

/// Initialize BetterAuth with the database pool and auth configuration.
pub async fn create_auth(pool: sqlx::PgPool, cfg: &AppConfig) -> anyhow::Result<AuthState> {
    let db = SqlxAdapter::from_pool(pool);
    let mut config = BetterAuthConfig::new(&cfg.secret).base_url(&cfg.base_url);

    for origin in &cfg.trusted_origins {
        config = config.trusted_origin(origin);
    }

    // C4: SameSite=Strict still permits same-site cross-port requests
    // (e.g. dashboard:3000 → api:8082 under the same eTLD+1), which matches
    // our deployment topology. `Secure` is gated by `tls_required` so dev
    // over plain HTTP can still receive and store the cookie.
    config.session.cookie_same_site = SameSite::Strict;
    if !cfg.tls_required {
        config.session.cookie_secure = false;
    }

    let mut email_plugin = EmailPasswordPlugin::new().enable_signup(true);
    if cfg.require_approval {
        email_plugin = email_plugin.auto_sign_in(false);
    }

    let auth = AuthBuilder::new(config)
        .database(db)
        .plugin(email_plugin)
        .plugin(SessionManagementPlugin::new())
        .plugin(ApiKeyPlugin::with_config(ApiKeyConfig::default()))
        .plugin(TwoFactorPlugin::new())
        .plugin(OrganizationPlugin::new())
        .plugin(AdminPlugin::new())
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to build BetterAuth: {}", e))?;

    Ok(Arc::new(auth))
}