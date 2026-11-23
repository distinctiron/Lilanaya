-- Logging in with a password. The emailed link stays for signing up and for choosing a new password.

-- An Argon2id hash with its own salt and settings (a PHC string). Null until the user has chosen a password.
ALTER TABLE users ADD COLUMN password_hash text;

-- Wrong passwords tried for an address lately, for the limit on guessing. A right one leaves no row.
CREATE TABLE log_in_attempts (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email      text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX log_in_attempts_email ON log_in_attempts (email, created_at);
