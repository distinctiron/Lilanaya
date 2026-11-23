-- Payment through Stripe: subscriptions, usage packs, and what a pack has paid for.

ALTER TABLE users
    -- The user's customer record at Stripe; made the first time they go to pay.
    ADD COLUMN stripe_customer_id text UNIQUE,
    -- When the user last agreed to the terms and to the service starting at once (asked before every payment).
    ADD COLUMN terms_accepted_at  timestamptz;

-- One row per user who has ever subscribed: the subscription at Stripe as last read from Stripe.
-- A user counts as a subscriber while `status` is alive and the paid period has not run out
-- (see `PAID_SUBSCRIPTION` in store.rs).
CREATE TABLE subscriptions (
    user_id                bigint PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    stripe_subscription_id text NOT NULL UNIQUE,
    -- Stripe's word: active, past_due, canceled, unpaid, incomplete, incomplete_expired, paused, trialing.
    status                 text NOT NULL,
    -- The latest period that has been PAID for. It moves forward only when Stripe says the invoice for the
    -- new period is paid, so a renewal that fails never opens a new allowance. Null until the first payment.
    period_start           timestamptz,
    period_end             timestamptz,
    -- The user has cancelled: the subscription ends at `period_end` and is not renewed.
    cancel_at_period_end   boolean NOT NULL DEFAULT false,
    updated_at             timestamptz NOT NULL DEFAULT now()
);

-- One row per usage pack bought. The pack's contents are copied here, so a later change of the catalog
-- does not change what was sold.
CREATE TABLE purchases (
    id                    bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id               bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The catalog's name of the pack: small, medium, large.
    pack                  text NOT NULL,
    strategies            bigint NOT NULL CHECK (strategies >= 0),
    runs                  bigint NOT NULL CHECK (runs >= 0),
    -- What was paid, in the currency's smallest unit (cents), tax included, as Stripe reported it.
    amount                bigint NOT NULL,
    currency              text NOT NULL,
    -- The Checkout session that paid for it: recording the same payment twice changes nothing.
    stripe_session_id     text NOT NULL UNIQUE,
    stripe_payment_intent text,
    created_at            timestamptz NOT NULL DEFAULT now(),
    -- What has been taken back out of the pack because money was paid back: the same share of the pack
    -- as the share of the price refunded. A pack paid back in full (or disputed) no longer counts at all.
    refunded_strategies   bigint NOT NULL DEFAULT 0,
    refunded_runs         bigint NOT NULL DEFAULT 0,
    -- When money was last paid back for it.
    refunded_at           timestamptz
);
CREATE INDEX purchases_user ON purchases (user_id, created_at);
CREATE INDEX purchases_payment_intent ON purchases (stripe_payment_intent);

-- Every message from Stripe that has been acted on, so one delivered twice is acted on once.
CREATE TABLE stripe_events (
    id          text PRIMARY KEY,
    kind        text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);

-- Paid for by a pack instead of the plan's own allowance. Such a row is left out when the plan's
-- allowance is counted, and counted against the packs instead. Deleting anything gives nothing back.
ALTER TABLE llm_calls ADD COLUMN from_pack boolean NOT NULL DEFAULT false;
ALTER TABLE runs      ADD COLUMN from_pack boolean NOT NULL DEFAULT false;
CREATE INDEX llm_calls_pack ON llm_calls (user_id) WHERE from_pack AND counted;
CREATE INDEX runs_pack ON runs (user_id) WHERE from_pack;

-- What a deleted account had used of the free strategies the AI writes, kept under a hash of its address
-- (not the address). Signing up again with the same address does not bring them back: each was a paid call.
CREATE TABLE spent_free_strategies (
    email_hash text NOT NULL,
    world_id   text NOT NULL,
    used       bigint NOT NULL,
    PRIMARY KEY (email_hash, world_id)
);
