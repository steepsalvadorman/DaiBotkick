ALTER TABLE channels ADD COLUMN IF NOT EXISTS queue_state TEXT NOT NULL DEFAULT '{"items":[],"version":0}';
ALTER TABLE channels ADD COLUMN IF NOT EXISTS show_video BOOLEAN NOT NULL DEFAULT TRUE;
CREATE TABLE IF NOT EXISTS webhook_events (
    message_id TEXT PRIMARY KEY,
    received_at BIGINT NOT NULL,
    payload TEXT NOT NULL,
    event_type TEXT NOT NULL,
    processed BOOLEAN NOT NULL DEFAULT FALSE
);

ALTER TABLE channels ADD COLUMN IF NOT EXISTS playback_token TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS webhook_events_pending ON webhook_events (processed, received_at, message_id);
