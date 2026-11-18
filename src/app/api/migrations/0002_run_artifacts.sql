-- The files each completed run writes to object storage (the run's manifest).

CREATE TABLE run_artifacts (
    run_id       bigint NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    name         text NOT NULL,     -- path inside the run's folder, e.g. run_summary.json or traces/a__b.json
    key          text NOT NULL,     -- object key in the bucket
    content_type text NOT NULL,
    size_bytes   bigint NOT NULL,
    sha256       text NOT NULL,     -- hex
    PRIMARY KEY (run_id, name)
);
