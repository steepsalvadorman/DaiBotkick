/**
 * Login para DaiBot via OAuth 2.0 de Kick.
 *
 * Uso:
 *   1. Pon KICK_CLIENT_ID y KICK_CLIENT_SECRET en .env
 *   2. Añade http://localhost:3001/callback como Redirect URL en kick.com/settings/developer
 *   3. Ejecuta: node login.js
 *   4. Autoriza en el navegador (incluye tu 2FA normal de Kick)
 *   5. Las tokens quedan guardadas en .env automáticamente
 */

import http      from 'http';
import crypto    from 'crypto';
import fs        from 'fs';
import path      from 'path';
import { exec }  from 'child_process';
import { fileURLToPath } from 'url';
import dotenv    from 'dotenv';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ENV_PATH  = path.resolve(__dirname, '..', '.env');

dotenv.config({ path: ENV_PATH });

// ── Configuración OAuth ───────────────────────────────────────────────────────

const CLIENT_ID     = process.env.KICK_CLIENT_ID;
const CLIENT_SECRET = process.env.KICK_CLIENT_SECRET;
const PORT          = 3001;
const REDIRECT_URI  = `http://localhost:${PORT}/callback`;

const SCOPES = [
    'user:read',
    'channel:read',
    'channel:update',
    'chat:write',
    'events:subscribe',
].join(' ');

// Endpoints oficiales — https://id.kick.com (no kick.com)
const AUTH_URL  = 'https://id.kick.com/oauth/authorize';
const TOKEN_URL = 'https://id.kick.com/oauth/token';

// ── Entrada principal ─────────────────────────────────────────────────────────

async function main() {
    if (!CLIENT_ID || !CLIENT_SECRET) {
        console.error('❌ Falta KICK_CLIENT_ID o KICK_CLIENT_SECRET en el .env');
        console.error('   Encuéntralos en: kick.com/settings/developer → tu app → Client ID / Secret');
        process.exit(1);
    }

    // PKCE — requerido por Kick
    const codeVerifier  = crypto.randomBytes(32).toString('base64url');
    const codeChallenge = crypto.createHash('sha256')
        .update(codeVerifier).digest('base64url');
    const state = crypto.randomBytes(16).toString('hex');

    console.log('🔐 Iniciando OAuth 2.0 con Kick (PKCE)...');
    console.log(`   Redirect: ${REDIRECT_URI}`);
    console.log('');

    // Escuchar el callback antes de abrir el navegador
    const { code } = await listenForCallback(state, codeVerifier);

    // Intercambiar code → tokens (PKCE requiere enviar el code_verifier)
    console.log('🔄 Intercambiando código por tokens...');
    const tokens = await exchangeCode(code, codeVerifier);

    // Guardar en .env
    saveTokens(tokens);
    console.log('');
    console.log('✅ Tokens guardados en .env');
    console.log('✅ Inicia el bot: cd backend && cargo run --release');
}

// ── Servidor local para el callback ──────────────────────────────────────────

function listenForCallback(expectedState, codeVerifier) {
    return new Promise((resolve, reject) => {
        const server = http.createServer((req, res) => {
            const url    = new URL(req.url, `http://localhost:${PORT}`);
            const code   = url.searchParams.get('code');
            const state  = url.searchParams.get('state');
            const error  = url.searchParams.get('error');

            const html = (body) => {
                res.setHeader('Content-Type', 'text/html; charset=utf-8');
                res.end(`<html><head><meta charset="utf-8"></head><body style="font-family:sans-serif;text-align:center;padding:40px;background:#0a0a0a;color:#fff">${body}</body></html>`);
            };

            if (error) {
                html('<h2>Error: ' + error + '</h2><p>Puedes cerrar esta pestaña.</p>');
                server.close();
                reject(new Error('OAuth rechazado: ' + error));
                return;
            }

            if (!code || state !== expectedState) {
                html('<h2>Parámetros inválidos</h2>');
                return;
            }

            html('<h2 style="color:#53fc18">✅ Autorizado correctamente</h2><p>Puedes cerrar esta pestaña y volver a la terminal.</p>');
            server.close();
            console.log('✅ Autorización recibida');
            resolve({ code, codeVerifier });
        });

        server.listen(PORT, () => {
            const codeChallenge = crypto.createHash('sha256')
                .update(codeVerifier).digest('base64url');
            const fullUrl = AUTH_URL + '?' + new URLSearchParams({
                client_id:             CLIENT_ID,
                redirect_uri:          REDIRECT_URI,
                response_type:         'code',
                scope:                 SCOPES,
                state:                 expectedState,
                code_challenge:        codeChallenge,
                code_challenge_method: 'S256',
            }).toString();
            console.log(`⏳ Esperando autorización en puerto ${PORT}...`);
            console.log('   Abriendo navegador...\n');
            console.log('   URL: ' + fullUrl + '\n');
            openBrowser(fullUrl);
        });

        server.on('error', reject);

        // Timeout de 5 minutos
        setTimeout(() => {
            server.close();
            reject(new Error('Timeout: no se recibió autorización en 5 minutos'));
        }, 5 * 60 * 1000);
    });
}

function openBrowser(url) {
    // Intenta abrir con xdg-open (Linux), open (macOS), start (Windows)
    const cmd = process.platform === 'darwin' ? `open "${url}"`
              : process.platform === 'win32'  ? `start "" "${url}"`
              : `xdg-open "${url}"`;
    exec(cmd, err => {
        if (err) {
            console.log('⚠️  No se pudo abrir el navegador automáticamente.');
            console.log('   Abre esta URL manualmente:\n');
            console.log('   ' + url + '\n');
        }
    });
}

// ── Intercambio de código ─────────────────────────────────────────────────────

async function exchangeCode(code, codeVerifier) {
    const body = new URLSearchParams({
        grant_type:    'authorization_code',
        client_id:     CLIENT_ID,
        client_secret: CLIENT_SECRET,
        redirect_uri:  REDIRECT_URI,
        code,
        code_verifier: codeVerifier,
    });

    const res = await fetch(TOKEN_URL, {
        method:  'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body:    body.toString(),
    });

    if (!res.ok) {
        const text = await res.text();
        throw new Error(`Token endpoint devolvió ${res.status}: ${text}`);
    }

    const data = await res.json();

    if (!data.access_token) {
        throw new Error('Respuesta sin access_token: ' + JSON.stringify(data));
    }

    console.log(`   access_token:  ${data.access_token.slice(0, 20)}...`);
    console.log(`   refresh_token: ${data.refresh_token?.slice(0, 20) ?? '(no incluido)'}...`);
    console.log(`   expires_in:    ${data.expires_in ?? '?'} segundos`);

    return data;
}

// ── Guardar en .env ───────────────────────────────────────────────────────────

function saveTokens(tokens) {
    let content = fs.existsSync(ENV_PATH) ? fs.readFileSync(ENV_PATH, 'utf8') : '';

    content = setKey(content, 'KICK_ACCESS_TOKEN',  tokens.access_token  ?? '');
    content = setKey(content, 'KICK_REFRESH_TOKEN', tokens.refresh_token ?? '');
    content = setKey(content, 'KICK_TOKEN_EXPIRES', String(
        tokens.expires_in ? Math.floor(Date.now() / 1000) + tokens.expires_in : 0
    ));

    fs.writeFileSync(ENV_PATH, content.trim() + '\n');
}

function setKey(content, key, value) {
    const escaped = value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    const line    = `${key}="${escaped}"`;
    const regex   = new RegExp(`^${key}=.*$`, 'm');
    return regex.test(content) ? content.replace(regex, line) : content + `\n${line}`;
}

main().catch(err => {
    console.error('❌ Error:', err.message);
    process.exit(1);
});
