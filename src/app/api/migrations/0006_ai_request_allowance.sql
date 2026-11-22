-- The plan's allowance of AI requests: every time the language model is asked to write rules, one is used up.
-- A free user has a few per world for good; a subscriber has a number per month across all worlds.

ALTER TABLE llm_calls
    -- The world the rules were asked for. Null on calls made before this allowance existed.
    ADD COLUMN world_id text,
    -- Uses up one of the user's AI requests. Set to false afterwards only when the language model was never
    -- reached (the screen turned the description down, or the provider did not answer). Deleting a strategy
    -- changes nothing here: the request was paid for.
    ADD COLUMN counted  boolean NOT NULL DEFAULT true;
CREATE INDEX llm_calls_allowance ON llm_calls (user_id, world_id) WHERE counted;
