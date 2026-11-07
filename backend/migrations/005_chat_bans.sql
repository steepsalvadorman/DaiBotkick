-- Usuarios del chat bloqueados por canal: solo pueden escribir texto, sin comandos ni funciones del overlay.
CREATE TABLE IF NOT EXISTS chat_bans (
    slug TEXT NOT NULL,
    username TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (slug, username)
);
