CREATE TABLE IF NOT EXISTS channels (
    slug                TEXT PRIMARY KEY,
    broadcaster_user_id BIGINT,
    chatroom_id         BIGINT,
    access_token        TEXT NOT NULL,
    refresh_token       TEXT NOT NULL DEFAULT '',
    token_expires       BIGINT NOT NULL DEFAULT 0,
    panel_token         TEXT NOT NULL DEFAULT '',
    cmd_discord         TEXT NOT NULL DEFAULT '',
    cmd_redes           TEXT NOT NULL DEFAULT '',
    cmd_pc              TEXT NOT NULL DEFAULT '',
    cmd_horario         TEXT NOT NULL DEFAULT '',
    follow_goal         BIGINT NOT NULL DEFAULT 100
);

CREATE TABLE IF NOT EXISTS oauth_state (
    token           TEXT PRIMARY KEY,
    code_verifier   TEXT NOT NULL,
    created_at      BIGINT NOT NULL
);
