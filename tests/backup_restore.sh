#!/usr/bin/env bash
# Exercise an archive round trip using disposable fixtures, never existing channel data.
set -euo pipefail
: "${TEST_DATABASE_URL:?Set TEST_DATABASE_URL to an isolated PostgreSQL test database}"
for tool in psql pg_dump pg_restore python3; do
    command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
suffix=$(python3 -c 'import uuid; print(uuid.uuid4().hex)')
schema="daibot_backup_$suffix"
restore_db="daibot_restore_$suffix"
restore_url=$(python3 - "$restore_db" <<'PY'
import os, sys
from urllib.parse import urlsplit, urlunsplit
url = urlsplit(os.environ['TEST_DATABASE_URL'])
if url.scheme not in ('postgresql', 'postgres'):
    raise SystemExit('TEST_DATABASE_URL must be a PostgreSQL URL')
print(urlunsplit(url._replace(path='/' + sys.argv[1])))
PY
)
temp_dir=$(mktemp -d)
schema_created=false
database_created=false
cleanup() {
    if "$database_created"; then
        psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 -c "DROP DATABASE $restore_db WITH (FORCE)" >/dev/null
    fi
    if "$schema_created"; then
        PGOPTIONS='-c client_min_messages=warning' psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 -c "DROP SCHEMA $schema CASCADE" >/dev/null
    fi
    rm -rf -- "$temp_dir"
}
trap cleanup EXIT
psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 -c "CREATE SCHEMA $schema"
schema_created=true
for migration in "$repo_dir"/backend/migrations/*.sql; do
    PGOPTIONS="-c search_path=$schema" psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 --single-transaction -f "$migration"
done
PGOPTIONS="-c search_path=$schema" psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO channels (slug, broadcaster_user_id, access_token, refresh_token,
 panel_token, playback_token, cmd_discord, follow_goal, show_video, queue_state)
VALUES ('alpha', 1, 'fixture-access', 'fixture-refresh', 'fixture-panel', 'fixture-playback',
 'saved-setting', 200, false,
 '{"items":[{"id":"persisted","url":"https://cdn.example/video.mp4","title":"saved","user":"viewer"}],"version":9}');
INSERT INTO webhook_events (message_id,received_at,payload,event_type)
VALUES ('pending',0,'{}','test.unknown');
SQL
pg_dump "$TEST_DATABASE_URL" --format=custom --schema="$schema" --file="$temp_dir/backup.dump"
psql "$TEST_DATABASE_URL" -Xq -v ON_ERROR_STOP=1 -c "CREATE DATABASE $restore_db TEMPLATE template0"
database_created=true
pg_restore --exit-on-error --dbname="$restore_url" "$temp_dir/backup.dump"
restored=$(PGOPTIONS="-c search_path=$schema" psql "$restore_url" -XAt -v ON_ERROR_STOP=1 <<'SQL'
SELECT slug,cmd_discord,follow_goal,show_video,
 queue_state::jsonb->'items'->0->>'id',queue_state::jsonb->>'version',
 access_token='fixture-access' AND refresh_token='fixture-refresh'
 AND panel_token='fixture-panel' AND playback_token='fixture-playback'
FROM channels;
SELECT count(*) FROM webhook_events WHERE NOT processed;
SQL
)
[[ "$restored" == $'alpha|saved-setting|200|f|persisted|9|t\n1' ]] || {
    echo 'Restored channel, queue or inbox differs from the fixture' >&2
    exit 1
}
echo 'Backup/restore passed: settings, tokens, queue IDs/version, visibility and pending inbox.'
