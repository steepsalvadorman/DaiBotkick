# DaiBot — Bot de Stream para Kick.com

Bot multi-tenant de streaming para Kick.com desplegado en la nube. Gestiona el chat, reproduce videos de YouTube pedidos por el chat, hace text-to-speech, maneja sorteos y muestra un overlay animado en OBS. Cualquier streamer puede conectar su canal sin necesidad de instalar nada.

> Desarrollado para el canal **SeniorDai** en Kick.com y disponible como servicio compartido.

---

## ¿Qué hace?

| Función | Descripción |
|---|---|
| 💬 Chat en vivo | Lee el chat de Kick vía EventSub webhook y responde a comandos |
| 🎬 Cola de videos | El chat pide videos de YouTube con `!play` |
| 🔊 Text-to-Speech | El chat hace hablar al bot con `!dai` y otras voces |
| 🎮 Entretenimiento | Dados, 8-ball y sorteos con `!dado`, `!8ball`, `!sorteo` |
| 📢 Respuestas automáticas | Discord, PC, horario, seguidores, uptime… |
| 🛡️ Anti-spam | Cooldowns por usuario y globales en todos los comandos |
| 📺 Overlay OBS | Pantalla animada estilo pixel art con chat y reproductor de video |
| 👥 Seguidores en tiempo real | Contador actualizado cada 60s vía API de Kick |
| 📱 Responsive | El overlay se adapta a cualquier resolución (1920×1080, móvil…) |

---

## Arquitectura

```
Kick EventSub Webhooks ──→ Render (Rust backend) ──→ Socket.IO ──→ OBS Overlay
Kick Pusher WebSocket  ──→       │
                                 │
                            PostgreSQL (Supabase)
                            · channels (tokens OAuth, IDs)
                            · oauth_state (PKCE flow)
```

- **Backend:** Rust — Axum + socketioxide + sqlx + reqwest
- **Base de datos:** Supabase PostgreSQL (Session Pooler, puerto 5432)
- **Deploy:** Render — Docker (plan Starter, siempre encendido)
- **Chat:** Kick EventSub webhooks + Pusher WebSocket (canal público)
- **TTS:** edge-tts (voces de Microsoft, instalado en el contenedor)

---

## Conectar tu canal

1. Ve a `https://daibotkick.onrender.com`
2. Haz clic en **Conectar con Kick**
3. Autoriza la app en Kick.com
4. Copia la URL del overlay que aparece en la pantalla de éxito

**Listo** — el bot empieza a leer tu chat al instante.

---

## Configurar OBS

Agrega una **Browser Source** con estos ajustes:

| Campo | Valor |
|---|---|
| URL | `https://daibotkick.onrender.com/pixel.html?ch=tu_slug` |
| Ancho | `1920` |
| Alto | `1080` |
| Controlar audio vía OBS | ✅ Marcado |

Sustituye `tu_slug` por tu nombre de canal en Kick (ej. `seniordai`).

> El overlay escala automáticamente a cualquier resolución — también funciona en móvil.

---

## Comandos del chat

### Cualquiera del chat

| Comando | Descripción | Cooldown |
|---|---|---|
| `!play [url]` | Agrega un video de YouTube a la cola | 30s por usuario |
| `!cola` | Muestra los próximos videos en cola | 30s global |
| `!quitarme` | Elimina tu primer video de la cola | — |
| `!misongs` | Ve tus videos con su posición en cola | 15s por usuario |
| `!dai [texto]` | TTS voz Camila (acento peruano) | 15s por usuario |
| `!dalia [texto]` | TTS voz Dalia (mexicano) | 15s por usuario |
| `!jorge [texto]` | TTS voz Jorge (mexicano) | 15s por usuario |
| `!alex [texto]` | TTS voz Alex (peruano) | 15s por usuario |
| `!dado` | Número aleatorio del 1 al 100 | 15s por usuario |
| `!8ball [pregunta]` | El oráculo responde | 10s por usuario |
| `!sorteo` / `!participar` | Entrar al sorteo cuando esté abierto | — |
| `!uptime` | Tiempo que llevamos en vivo | 30s global |
| `!seguidores` | Seguidores actuales vs meta | 60s global |
| `!discord` | Link al servidor de Discord | 20s global |
| `!redes` | Redes sociales | 20s global |
| `!pc` / `!setup` | Especificaciones del equipo | 20s global |
| `!horario` | Horario de streams | 20s global |
| `!comandos` / `!help` | Lista rápida de comandos | 20s global |

> Todos los comandos TTS (`!dai`, `!dalia`, `!jorge`, `!alex`) comparten el mismo cooldown de 15s.

### Solo el streamer

| Comando | Descripción |
|---|---|
| `!von` | Muestra el widget de video en el overlay |
| `!voff` | Oculta el video (el audio continúa) |
| `!vstop` | Para el video y vacía la cola |
| `!next` / `!skip` | Salta al siguiente video |
| `!sorteo abrir` | Abre el sorteo para participantes |
| `!sorteo cerrar` | Cierra el sorteo |
| `!sorteo ganador` | Elige y anuncia al ganador al azar |

---

## Para desarrolladores

### Requisitos locales

- [Rust](https://rustup.rs) (toolchain stable)
- PostgreSQL o cadena de conexión a Supabase
- Python 3 + `pip install edge-tts` (para TTS)

### Variables de entorno

```env
KICK_CLIENT_ID=tu_client_id
KICK_CLIENT_SECRET=tu_client_secret
DATABASE_URL=postgresql://postgres.xxx:password@aws-1-us-east-2.pooler.supabase.com:5432/postgres
BASE_URL=https://daibotkick.onrender.com
OVERLAY_DIR=../overlay
TTS_CACHE_DIR=/tmp/tts_cache
PORT=3000
```

Crea la app OAuth en [kick.com/settings/developer](https://kick.com/settings/developer) con:
- **Redirect URL:** `https://daibotkick.onrender.com/auth/callback`
- **Webhook URL:** `https://daibotkick.onrender.com/kick_webhook`

### Ejecutar en local

```bash
cd backend
cargo run
```

### Tests

```bash
cd backend
cargo test
```

43 tests unitarios: cooldowns, cola de videos, voces TTS, parsing de URLs de YouTube y helpers de configuración.

### Deploy en Render

El archivo `render.yaml` configura el servicio. Render usa el `Dockerfile` que:
1. Compila el backend con Rust 1.88
2. Instala Python 3 + edge-tts en la imagen final
3. Sirve el overlay desde `/app/overlay/`

Variables de entorno requeridas en el dashboard de Render:
- `KICK_CLIENT_ID`
- `KICK_CLIENT_SECRET`
- `DATABASE_URL` (Session Pooler de Supabase, puerto 5432)
- `BASE_URL` (URL pública del servicio)

---

## Estructura del proyecto

```
DaiBotkick/
├── .env.example            ← Plantilla de configuración
├── Dockerfile              ← Imagen Docker para Render
├── render.yaml             ← Configuración de Render
│
├── backend/                ← Servidor en Rust (axum + socketioxide + sqlx)
│   └── src/
│       ├── main.rs         ← Punto de entrada, AppState, router HTTP, webhook
│       ├── auth.rs         ← OAuth 2.0 PKCE (registro de streamers)
│       ├── channel.rs      ← Inicialización de canales por streamer
│       ├── commands/       ← Lógica de todos los comandos del chat
│       ├── cooldown.rs     ← Anti-spam: cooldowns por usuario y globales
│       ├── db.rs           ← Queries PostgreSQL (upsert, load, tokens)
│       ├── kick/           ← Pusher WebSocket + EventSub + sender
│       ├── tts/            ← Text-to-speech vía edge-tts
│       ├── queue/          ← Cola de videos
│       ├── server/         ← Socket.IO con el overlay (rooms por canal)
│       ├── state.rs        ← AppState, ChannelState, tipos compartidos
│       └── stats/          ← Viewer count y followers vía API de Kick
│
├── overlay/                ← Archivos servidos como static files
│   └── pixel.html          ← Overlay principal (pixel art, chat, reproductor)
│
└── installer/              ← Empaquetado legacy para Windows (no activo)
    ├── DaiBot.iss          ← Script de Inno Setup 6
    └── build.ps1           ← Build script
```

---

## Solución de problemas

**El overlay muestra viewers/seguidores en 0**
→ Los datos se actualizan en el primer fetch al iniciar el canal (~2s) y luego cada 60s
→ Si persiste, verifica que el token OAuth esté vigente

**El bot no responde a comandos**
→ Verifica en los logs de Render que aparezca `[EventSub][slug] Suscripciones creadas`
→ El bot lee el chat vía webhook, no hace polling

**El overlay no se ve en OBS**
→ Verifica que la URL incluya `?ch=tu_slug`
→ Refresca la browser source en OBS (botón derecho → Refresh)

**No se escucha el TTS**
→ Verifica que "Controlar audio vía OBS" esté marcado en la browser source

**El video no reproduce**
→ Usa una sola browser source en OBS
→ En algunos navegadores el autoplay requiere interacción previa — haz clic en el overlay una vez
