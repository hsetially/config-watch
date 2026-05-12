-- Better Auth RS core tables (v0.10 schema)
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    name TEXT,
    username TEXT,
    display_username TEXT,
    image TEXT,
    role TEXT,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    two_factor_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    banned BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason TEXT,
    ban_expires TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token TEXT UNIQUE NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    ip_address TEXT,
    user_agent TEXT,
    impersonated_by TEXT,
    active_organization_id TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    access_token TEXT,
    refresh_token TEXT,
    access_token_expires_at TIMESTAMPTZ,
    refresh_token_expires_at TIMESTAMPTZ,
    scope TEXT,
    id_token TEXT,
    password TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS verifications (
    id TEXT PRIMARY KEY,
    identifier TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Better Auth RS organization plugin tables (singular names matching crate expectations)
CREATE TABLE IF NOT EXISTS organization (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    logo TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS member (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organization(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, organization_id)
);

CREATE TABLE IF NOT EXISTS invitation (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organization(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member',
    status TEXT NOT NULL DEFAULT 'pending',
    expires_at TIMESTAMPTZ NOT NULL,
    inviter_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Better Auth RS API key plugin table
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    name TEXT,
    start TEXT,
    prefix TEXT NOT NULL,
    "key" TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    refill_interval INTEGER,
    refill_amount INTEGER,
    last_refill_at TIMESTAMPTZ,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    rate_limit_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    rate_limit_time_window INTEGER,
    rate_limit_max INTEGER,
    request_count INTEGER NOT NULL DEFAULT 0,
    remaining INTEGER NOT NULL DEFAULT 0,
    last_request TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    permissions TEXT,
    metadata TEXT,
    organization_id TEXT REFERENCES organization(id) ON DELETE CASCADE
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_accounts_user_id ON accounts(user_id);
CREATE INDEX IF NOT EXISTS idx_accounts_provider ON accounts(provider_id, account_id);
CREATE INDEX IF NOT EXISTS idx_verifications_identifier ON verifications(identifier);
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(prefix);
CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX IF NOT EXISTS idx_member_org_id ON member(organization_id);
CREATE INDEX IF NOT EXISTS idx_invitation_org_id ON invitation(organization_id);