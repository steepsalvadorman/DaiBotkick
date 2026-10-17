-- Retira la integración Steam/L4D2 y sus datos personales (perfiles, sesiones y asociaciones).
DROP TABLE IF EXISTS l4d2_links;
DROP TABLE IF EXISTS l4d2_settings;
DROP TABLE IF EXISTS steam_sessions;
DROP TABLE IF EXISTS steam_profiles;
DROP TABLE IF EXISTS steam_pending;
