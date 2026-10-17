CREATE TABLE steam_pending (
    state TEXT PRIMARY KEY,
    channel TEXT NOT NULL REFERENCES channels(slug) ON DELETE CASCADE,
    expires BIGINT NOT NULL
);
CREATE TABLE steam_profiles (
    channel TEXT NOT NULL REFERENCES channels(slug) ON DELETE CASCADE,
    steamid TEXT NOT NULL,
    profile TEXT NOT NULL,
    updated BIGINT NOT NULL,
    PRIMARY KEY (channel, steamid)
);
CREATE TABLE steam_sessions (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    steamid TEXT NOT NULL,
    expires BIGINT NOT NULL,
    FOREIGN KEY (channel, steamid) REFERENCES steam_profiles(channel, steamid) ON DELETE CASCADE
);
CREATE TABLE l4d2_settings (
    channel TEXT PRIMARY KEY REFERENCES channels(slug) ON DELETE CASCADE,
    server TEXT NOT NULL DEFAULT '',
    snapshot TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE l4d2_links (
    channel TEXT NOT NULL,
    player_name TEXT NOT NULL,
    steamid TEXT NOT NULL,
    PRIMARY KEY (channel, player_name),
    FOREIGN KEY (channel, steamid) REFERENCES steam_profiles(channel, steamid) ON DELETE CASCADE
);
CREATE INDEX steam_pending_expiry ON steam_pending(expires);
CREATE INDEX steam_sessions_expiry ON steam_sessions(expires);
