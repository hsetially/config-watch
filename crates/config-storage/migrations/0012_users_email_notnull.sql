-- H7 fix: Make email NOT NULL (nullable UNIQUE allows duplicate NULL emails)
-- and add partial indexes for role lookups used by the admin approval flow.

-- Step 1: Set email to a placeholder for any rows where email IS NULL.
-- This should only affect edge-case rows from the initial migration.
UPDATE users SET email = CONCAT('user-', id, '@placeholder.local') WHERE email IS NULL;

-- Step 2: Make email NOT NULL.
ALTER TABLE users ALTER COLUMN email SET NOT NULL;

-- Step 3: Add partial index for pending_approval role lookups (routes.rs:1887).
CREATE INDEX IF NOT EXISTS idx_users_role_pending ON users(role) WHERE role = 'pending_approval';

-- Step 4: Add general partial index for non-NULL roles.
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role) WHERE role IS NOT NULL;