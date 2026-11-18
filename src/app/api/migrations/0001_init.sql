-- Worlds, strategies, runs (which double as the job queue), per-pairing traces and KPI snapshots.

CREATE TABLE worlds (
    id         text PRIMARY KEY,
    doc        jsonb NOT NULL,                 -- the world document (engine schema v1)
    builtin    boolean NOT NULL DEFAULT false, -- templates shipped with the code, refreshed at startup
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE strategies (
    id         text PRIMARY KEY,
    position   integer NOT NULL DEFAULT 0,     -- display order
    doc        jsonb NOT NULL,                 -- the finite state machine
    builtin    boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE runs (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    world_id    text NOT NULL REFERENCES worlds (id),
    world_name  text NOT NULL,
    status      text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    seed        bigint NOT NULL,
    iterations  integer NOT NULL,
    actor_count integer NOT NULL,
    -- Everything needed to reproduce the run, frozen when it was queued: {world, actors, seed}.
    input       jsonb NOT NULL,
    result      jsonb,                          -- the engine's run summary, once completed
    error       jsonb,                          -- {message, details}, if failed
    created_at  timestamptz NOT NULL DEFAULT now(),
    started_at  timestamptz,
    finished_at timestamptz
);

-- The worker takes the oldest queued run.
CREATE INDEX runs_queued ON runs (id) WHERE status = 'queued';
-- Leaderboards and KPI history read the latest completed runs of a world.
CREATE INDEX runs_world_completed ON runs (world_id, id DESC) WHERE status = 'completed';

CREATE TABLE run_traces (
    run_id bigint NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    a      text NOT NULL,                       -- actor ids; a sorts before b
    b      text NOT NULL,
    moves  bytea NOT NULL,                      -- two bytes per round: a's decision index, b's decision index
    PRIMARY KEY (run_id, a, b)
);

CREATE TABLE kpi_snapshots (
    run_id   bigint NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    world_id text NOT NULL REFERENCES worlds (id),
    kpi_id   text NOT NULL,
    label    text NOT NULL,
    value    double precision NOT NULL,
    PRIMARY KEY (run_id, kpi_id)
);

CREATE INDEX kpi_snapshots_world ON kpi_snapshots (world_id, run_id);
