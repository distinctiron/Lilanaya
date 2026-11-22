-- Sign-in through an account somewhere else (Google, Apple, Facebook, GitHub).
-- One row per outside account that has signed in: the provider's own id for it, and the user it signs in as.
-- A user can have several (one address used at two providers), and still signs in by emailed link as before.
CREATE TABLE user_identities (
    provider   text NOT NULL CHECK (provider IN ('google', 'apple', 'facebook', 'github')),
    subject    text NOT NULL,                           -- the provider's stable id for the account
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX user_identities_user ON user_identities (user_id);
