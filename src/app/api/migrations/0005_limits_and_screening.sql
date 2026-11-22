-- Usage limits, what the screening step said about each compile, and the end of stored run files.

-- Run files (summary, matrix, traces, manifest) are now built on request from `runs` and `run_traces`.
DROP TABLE run_artifacts;

-- The cap on runs a user has queued or running.
CREATE INDEX runs_user_active ON runs (user_id) WHERE status IN ('queued', 'running');

-- One row per strategy check (a preview or a save), for the hourly limit.
CREATE TABLE preview_calls (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX preview_calls_user ON preview_calls (user_id, created_at);

ALTER TABLE llm_calls
    -- What the screen made of the description: 'strategy_description', 'instructions_to_the_ai', …; null if it was never asked.
    ADD COLUMN verdict       text,
    ADD COLUMN confidence    double precision,
    -- The description was turned down by the screen and never reached the language model.
    ADD COLUMN blocked       boolean NOT NULL DEFAULT false,
    -- Tokens used by the screen and the language model together, for the daily ceiling.
    ADD COLUMN input_tokens  bigint NOT NULL DEFAULT 0,
    ADD COLUMN output_tokens bigint NOT NULL DEFAULT 0;
-- The daily ceiling adds up every user's calls of the day.
CREATE INDEX llm_calls_day ON llm_calls (created_at);
