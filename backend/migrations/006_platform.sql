-- Canales por plataforma: un canal de Kick y uno de Twitch con el mismo nombre conviven.
-- Las filas existentes son de Kick, así que ningún canal tiene que volver a conectarse.

ALTER TABLE channels ADD COLUMN IF NOT EXISTS platform TEXT NOT NULL DEFAULT 'kick'
    CONSTRAINT channels_platform_check CHECK (platform IN ('kick', 'twitch'));
ALTER TABLE channels DROP CONSTRAINT channels_pkey;
ALTER TABLE channels ADD CONSTRAINT channels_pkey PRIMARY KEY (platform, slug);

-- Un baneo pertenece al canal de una plataforma.
ALTER TABLE chat_bans ADD COLUMN IF NOT EXISTS platform TEXT NOT NULL DEFAULT 'kick'
    CONSTRAINT chat_bans_platform_check CHECK (platform IN ('kick', 'twitch'));
ALTER TABLE chat_bans DROP CONSTRAINT chat_bans_pkey;
ALTER TABLE chat_bans ADD CONSTRAINT chat_bans_pkey PRIMARY KEY (platform, slug, username);

-- El inbox guarda eventos de las dos plataformas; cada una asigna sus propios IDs de entrega.
ALTER TABLE webhook_events ADD COLUMN IF NOT EXISTS platform TEXT NOT NULL DEFAULT 'kick'
    CONSTRAINT webhook_events_platform_check CHECK (platform IN ('kick', 'twitch'));
ALTER TABLE webhook_events DROP CONSTRAINT webhook_events_pkey;
ALTER TABLE webhook_events ADD CONSTRAINT webhook_events_pkey PRIMARY KEY (platform, message_id);

-- Cuenta que escribe en el chat de todos los canales de una plataforma (en Twitch, DaiBotsito).
CREATE TABLE IF NOT EXISTS bot_accounts (
    platform      TEXT PRIMARY KEY CHECK (platform IN ('kick', 'twitch')),
    user_id       TEXT NOT NULL,
    login         TEXT NOT NULL,
    access_token  TEXT NOT NULL,
    refresh_token TEXT NOT NULL DEFAULT '',
    token_expires BIGINT NOT NULL DEFAULT 0,
    scopes        TEXT NOT NULL DEFAULT '',
    updated_at    BIGINT NOT NULL
);
