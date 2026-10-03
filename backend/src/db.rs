use sqlx::PgPool;

const COLUMNS: &str = "slug, broadcaster_user_id, chatroom_id, access_token, refresh_token, token_expires, panel_token, cmd_discord, cmd_redes, cmd_pc, cmd_horario, follow_goal, queue_state, show_video, playback_token";

#[derive(sqlx::FromRow, Clone)]
pub struct ChannelRow {
    pub slug: String,
    pub broadcaster_user_id: Option<i64>,
    pub chatroom_id: Option<i64>,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expires: i64,
    pub panel_token: String,
    pub cmd_discord: String,
    pub cmd_redes: String,
    pub cmd_pc: String,
    pub cmd_horario: String,
    pub follow_goal: i64,
    pub queue_state: String,
    pub show_video: bool,
    pub playback_token: String,
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

pub async fn load_all_channels(pool: &PgPool) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(&format!("SELECT {COLUMNS} FROM channels"))
        .fetch_all(pool)
        .await
}

pub async fn load_channel(pool: &PgPool, slug: &str) -> Result<ChannelRow, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(&format!("SELECT {COLUMNS} FROM channels WHERE slug=$1"))
        .bind(slug)
        .fetch_one(pool)
        .await
}

pub async fn upsert_channel(pool: &PgPool, row: &ChannelRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channels
            (slug, broadcaster_user_id, chatroom_id, access_token, refresh_token,
             token_expires, panel_token, cmd_discord, cmd_redes, cmd_pc, cmd_horario, follow_goal, playback_token)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         ON CONFLICT (slug) DO UPDATE SET
           broadcaster_user_id = COALESCE(EXCLUDED.broadcaster_user_id, channels.broadcaster_user_id),
           chatroom_id         = COALESCE(EXCLUDED.chatroom_id, channels.chatroom_id),
           access_token        = EXCLUDED.access_token,
           refresh_token       = EXCLUDED.refresh_token,
           token_expires       = EXCLUDED.token_expires,
           playback_token      = CASE WHEN channels.playback_token = '' THEN EXCLUDED.playback_token ELSE channels.playback_token END,
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
    .bind(&row.playback_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Guarda un texto de comando configurable (!discord, !redes, !pc, !horario).
pub async fn update_command_text(
    pool: &PgPool,
    slug: &str,
    command: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    let column = match command {
        "discord" => "cmd_discord",
        "redes" => "cmd_redes",
        "pc" => "cmd_pc",
        "horario" => "cmd_horario",
        _ => return Err(sqlx::Error::ColumnNotFound(command.to_string())),
    };
    sqlx::query(&format!("UPDATE channels SET {column}=$1 WHERE slug=$2"))
        .bind(value)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_follow_goal(pool: &PgPool, slug: &str, goal: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET follow_goal=$1 WHERE slug=$2")
        .bind(goal)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_tokens(
    pool: &PgPool,
    slug: &str,
    access: &str,
    refresh: &str,
    expires: i64,
    previous_refresh: &str,
) -> Result<bool, sqlx::Error> {
    let updated = sqlx::query(
        "UPDATE channels SET access_token=$1, refresh_token=$2, token_expires=$3 WHERE slug=$4 AND refresh_token=$5",
    )
    .bind(access)
    .bind(refresh)
    .bind(expires)
    .bind(slug)
    .bind(previous_refresh)
    .execute(pool)
    .await?;
    Ok(updated.rows_affected() == 1)
}

pub async fn save_oauth_state(
    pool: &PgPool,
    token: &str,
    code_verifier: &str,
) -> Result<(), sqlx::Error> {
    let now = unix_now() as i64;
    sqlx::query(
        "INSERT INTO oauth_state (token, code_verifier, created_at) VALUES ($1,$2,$3)
         ON CONFLICT (token) DO UPDATE SET code_verifier=EXCLUDED.code_verifier, created_at=EXCLUDED.created_at",
    )
    .bind(token)
    .bind(code_verifier)
    .bind(now)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM oauth_state WHERE created_at < $1")
        .bind(now - 600)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn consume_oauth_state(
    pool: &PgPool,
    token: &str,
) -> Result<Option<String>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        code_verifier: String,
        created_at: i64,
    }

    let row = sqlx::query_as::<_, Row>(
        "DELETE FROM oauth_state WHERE token=$1 RETURNING code_verifier, created_at",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if unix_now() as i64 - row.created_at > 600 {
        return Ok(None);
    }
    Ok(Some(row.code_verifier))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
