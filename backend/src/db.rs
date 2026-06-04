use sqlx::PgPool;

#[derive(sqlx::FromRow, Clone)]
pub struct ChannelRow {
    pub slug:                String,
    pub broadcaster_user_id: Option<i64>,
    pub chatroom_id:         Option<i64>,
    pub access_token:        String,
    pub refresh_token:       String,
    pub token_expires:       i64,
    pub panel_token:         String,
    pub cmd_discord:         String,
    pub cmd_redes:           String,
    pub cmd_pc:              String,
    pub cmd_horario:         String,
    pub follow_goal:         i64,
}

pub async fn run_migrations(pool: &PgPool) {
    let sql = include_str!("../migrations/001_initial.sql");
    for stmt in sql.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if let Err(e) = sqlx::query(stmt).execute(pool).await {
            tracing::warn!("[DB] Migration warning: {e}");
        }
    }
}

pub async fn load_all_channels(pool: &PgPool) -> Vec<ChannelRow> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT slug, broadcaster_user_id, chatroom_id, access_token, refresh_token,
                token_expires, panel_token, cmd_discord, cmd_redes, cmd_pc, cmd_horario, follow_goal
         FROM channels",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

pub async fn upsert_channel(pool: &PgPool, row: &ChannelRow) {
    let _ = sqlx::query(
        "INSERT INTO channels
            (slug, broadcaster_user_id, chatroom_id, access_token, refresh_token,
             token_expires, panel_token, cmd_discord, cmd_redes, cmd_pc, cmd_horario, follow_goal)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         ON CONFLICT (slug) DO UPDATE SET
           broadcaster_user_id = COALESCE(EXCLUDED.broadcaster_user_id, channels.broadcaster_user_id),
           chatroom_id         = COALESCE(EXCLUDED.chatroom_id, channels.chatroom_id),
           access_token        = EXCLUDED.access_token,
           refresh_token       = EXCLUDED.refresh_token,
           token_expires       = EXCLUDED.token_expires,
           panel_token         = CASE WHEN channels.panel_token = '' THEN EXCLUDED.panel_token ELSE channels.panel_token END,
           cmd_discord         = CASE WHEN EXCLUDED.cmd_discord  != '' THEN EXCLUDED.cmd_discord  ELSE channels.cmd_discord  END,
           cmd_redes           = CASE WHEN EXCLUDED.cmd_redes    != '' THEN EXCLUDED.cmd_redes    ELSE channels.cmd_redes    END,
           cmd_pc              = CASE WHEN EXCLUDED.cmd_pc       != '' THEN EXCLUDED.cmd_pc       ELSE channels.cmd_pc       END,
           cmd_horario         = CASE WHEN EXCLUDED.cmd_horario  != '' THEN EXCLUDED.cmd_horario  ELSE channels.cmd_horario  END,
           follow_goal         = CASE WHEN EXCLUDED.follow_goal  != 100 THEN EXCLUDED.follow_goal ELSE channels.follow_goal  END",
    )
    .bind(&row.slug)
    .bind(row.broadcaster_user_id)
    .bind(row.chatroom_id)
    .bind(&row.access_token)
    .bind(&row.refresh_token)
    .bind(row.token_expires)
    .bind(&row.panel_token)
    .bind(&row.cmd_discord)
    .bind(&row.cmd_redes)
    .bind(&row.cmd_pc)
    .bind(&row.cmd_horario)
    .bind(row.follow_goal)
    .execute(pool)
    .await;
}

pub async fn update_tokens(pool: &PgPool, slug: &str, access: &str, refresh: &str, expires: i64) {
    let _ = sqlx::query(
        "UPDATE channels SET access_token=$1, refresh_token=$2, token_expires=$3 WHERE slug=$4",
    )
    .bind(access)
    .bind(refresh)
    .bind(expires)
    .bind(slug)
    .execute(pool)
    .await;
}

pub async fn update_channel_ids(pool: &PgPool, slug: &str, broadcaster_id: i64, chatroom_id: i64) {
    let _ = sqlx::query(
        "UPDATE channels SET broadcaster_user_id=$1, chatroom_id=$2 WHERE slug=$3",
    )
    .bind(broadcaster_id)
    .bind(chatroom_id)
    .bind(slug)
    .execute(pool)
    .await;
}

pub async fn save_oauth_state(pool: &PgPool, token: &str, code_verifier: &str) {
    let now = unix_now() as i64;
    if let Err(e) = sqlx::query(
        "INSERT INTO oauth_state (token, code_verifier, created_at) VALUES ($1,$2,$3)
         ON CONFLICT (token) DO UPDATE SET code_verifier=EXCLUDED.code_verifier, created_at=EXCLUDED.created_at",
    )
    .bind(token)
    .bind(code_verifier)
    .bind(now)
    .execute(pool)
    .await {
        tracing::error!("[DB] save_oauth_state falló: {e}");
    }
}

pub async fn consume_oauth_state(pool: &PgPool, token: &str) -> Option<String> {
    #[derive(sqlx::FromRow)]
    struct Row { code_verifier: String, created_at: i64 }

    let row = sqlx::query_as::<_, Row>(
        "SELECT code_verifier, created_at FROM oauth_state WHERE token=$1",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .ok()??;

    let _ = sqlx::query("DELETE FROM oauth_state WHERE token=$1")
        .bind(token)
        .execute(pool)
        .await;

    if unix_now() as i64 - row.created_at > 600 { return None; }
    Some(row.code_verifier)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
