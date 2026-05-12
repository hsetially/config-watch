-- Track last login information per user
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_login_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_login_ip TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_login_user_agent TEXT;