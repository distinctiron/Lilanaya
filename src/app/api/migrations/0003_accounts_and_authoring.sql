-- Accounts, ownership of strategies and runs, and what strategy authoring needs.

CREATE TABLE users (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email        text NOT NULL UNIQUE,                  -- stored lower-case
    display_name text NOT NULL,
    roles        text[] NOT NULL DEFAULT '{user}',
    -- Decides the entitlement flags. Set by hand until billing exists.
    plan         text NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'subscriber')),
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- One-time sign-in links sent by email. Only the hash of the token is kept.
CREATE TABLE sign_in_tokens (
    token_hash text PRIMARY KEY,
    email      text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);
CREATE INDEX sign_in_tokens_email ON sign_in_tokens (email, created_at);

CREATE TABLE sessions (
    token_hash text PRIMARY KEY,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

ALTER TABLE worlds
    ADD COLUMN owner_id   bigint REFERENCES users (id),
    ADD COLUMN visibility text NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'private'));

ALTER TABLE strategies
    ADD COLUMN owner_id   bigint REFERENCES users (id) ON DELETE CASCADE,
    ADD COLUMN visibility text NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'private')),
    -- How it is authored: 'rules' (a state machine) or 'code' (JavaScript). Fixed once saved.
    ADD COLUMN mode       text NOT NULL DEFAULT 'rules' CHECK (mode IN ('rules', 'code')),
    -- The world whose decisions it was checked against.
    ADD COLUMN world_id   text NOT NULL DEFAULT 'prisoners-dilemma' REFERENCES worlds (id),
    -- The plain-English description the rules were generated from, if any.
    ADD COLUMN prompt     text;
CREATE INDEX strategies_owner ON strategies (owner_id);

ALTER TABLE runs
    ADD COLUMN user_id bigint REFERENCES users (id),
    -- A run that uses a private strategy or world: visible to its owner only, kept out of world analytics.
    ADD COLUMN private boolean NOT NULL DEFAULT false;
CREATE INDEX runs_user_month ON runs (user_id, created_at);

-- One row per plain-English compile, for the hourly limit and cost follow-up.
CREATE TABLE llm_calls (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX llm_calls_user ON llm_calls (user_id, created_at);
