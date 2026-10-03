#!/usr/bin/env bash
# Arranca GorilinRix en local con un túnel HTTPS público (necesario para los webhooks de Kick).
#
# Uso:  ./dev.sh
#
# Modos (se elige según lo que haya en .env):
#   NGROK_DOMAIN=mi-bot.ngrok-free.app      → ngrok con dominio fijo (gratis, 1 por cuenta)
#   CF_TUNNEL_NAME=daibot-dev               → túnel con nombre de cloudflared en tu dominio
#   CF_TUNNEL_URL=https://dev.midominio.com   (URL pública de ese túnel)
#   (ninguno)                               → túnel rápido de cloudflared; la URL cambia cada vez
#
# Con URL fija configuras la app de Kick una sola vez. Con el túnel rápido el script
# te indica qué URLs pegar en Kick en cada arranque.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$ROOT/.env"
PORT="${PORT:-3000}"
TUNNEL_LOG="$(mktemp -t daibot-tunnel.XXXXXX)"
TUNNEL_PID=""

env_get() { grep -E "^$1=" "$ENV_FILE" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '"' || true; }
env_set() {
    if grep -qE "^$1=" "$ENV_FILE"; then sed -i "s#^$1=.*#$1=$2#" "$ENV_FILE"; else echo "$1=$2" >> "$ENV_FILE"; fi
}
need() { command -v "$1" >/dev/null 2>&1 || { echo "Falta '$1'. $2" >&2; exit 1; }; }
cleanup() {
    [ -n "$TUNNEL_PID" ] && kill "$TUNNEL_PID" 2>/dev/null || true
    rm -f "$TUNNEL_LOG"
}
trap cleanup EXIT INT TERM

[ -f "$ENV_FILE" ] || { echo "No existe $ENV_FILE (copia .env.example)." >&2; exit 1; }
export PATH="$HOME/.local/bin:$PATH"

# Liberar el puerto si quedó un GorilinRix anterior abierto
if command -v fuser >/dev/null && fuser "$PORT/tcp" >/dev/null 2>&1; then
    echo "Cerrando el proceso que ocupaba el puerto $PORT..."
    fuser -k "$PORT/tcp" >/dev/null 2>&1 || true
    sleep 1
fi

NGROK_DOMAIN="$(env_get NGROK_DOMAIN)"
CF_TUNNEL_NAME="$(env_get CF_TUNNEL_NAME)"
CF_TUNNEL_URL="$(env_get CF_TUNNEL_URL)"
FIXED=1

if [ -n "$NGROK_DOMAIN" ]; then
    need ngrok "Instálalo desde https://ngrok.com/download y ejecuta 'ngrok config add-authtoken <token>'."
    PUBLIC_URL="https://${NGROK_DOMAIN#https://}"
    ngrok http --url="$PUBLIC_URL" "$PORT" --log=stdout >"$TUNNEL_LOG" 2>&1 &
    TUNNEL_PID=$!
elif [ -n "$CF_TUNNEL_NAME" ]; then
    need cloudflared "Descárgalo de https://github.com/cloudflare/cloudflared/releases"
    [ -n "$CF_TUNNEL_URL" ] || { echo "Define CF_TUNNEL_URL en .env" >&2; exit 1; }
    PUBLIC_URL="${CF_TUNNEL_URL%/}"
    cloudflared tunnel run --url "http://localhost:$PORT" "$CF_TUNNEL_NAME" >"$TUNNEL_LOG" 2>&1 &
    TUNNEL_PID=$!
else
    need cloudflared "Descárgalo de https://github.com/cloudflare/cloudflared/releases"
    FIXED=0
    cloudflared tunnel --url "http://localhost:$PORT" >"$TUNNEL_LOG" 2>&1 &
    TUNNEL_PID=$!
    echo "Abriendo túnel rápido de cloudflared..."
    PUBLIC_URL=""
    for _ in $(seq 1 30); do
        PUBLIC_URL="$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "$TUNNEL_LOG" | head -1 || true)"
        [ -n "$PUBLIC_URL" ] && break
        kill -0 "$TUNNEL_PID" 2>/dev/null || break
        sleep 1
    done
    [ -n "$PUBLIC_URL" ] || { echo "No se pudo abrir el túnel:" >&2; cat "$TUNNEL_LOG" >&2; exit 1; }
fi

sleep 2
kill -0 "$TUNNEL_PID" 2>/dev/null || { echo "El túnel se cerró:" >&2; cat "$TUNNEL_LOG" >&2; exit 1; }

PREVIOUS_URL="$(env_get BASE_URL)"
env_set BASE_URL "$PUBLIC_URL"

echo
echo "════════════════════════════════════════════════════════════════"
echo " Túnel activo: $PUBLIC_URL"
if [ "$FIXED" = 0 ] && [ "$PREVIOUS_URL" != "$PUBLIC_URL" ]; then
    echo
    echo " ⚠ La URL cambió. En kick.com/settings/developer → tu app Dev → Edit:"
    echo "     Redirect URL : $PUBLIC_URL/auth/callback"
    echo "     Webhook URL  : $PUBLIC_URL/kick_webhook"
    echo "   (o usa NGROK_DOMAIN en .env para tener una URL fija)"
fi
echo
echo " Conectar canal : $PUBLIC_URL/auth/kick"
echo " Detener        : Ctrl+C (cierra bot y túnel)"
echo "════════════════════════════════════════════════════════════════"
echo

cd "$ROOT/backend"
cargo run
