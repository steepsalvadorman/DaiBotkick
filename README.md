# DaiBot — Bot de Stream para Kick.com

Bot de streaming para Kick.com distribuible como instalador de Windows. Gestiona el chat, reproduce videos de YouTube pedidos por el chat, hace text-to-speech, maneja sorteos y muestra un overlay animado en OBS.

> Desarrollado para el canal **SeniorDai** en Kick.com pero distribuible a cualquier streamer.

---

## ¿Qué hace?

| Función | Descripción |
|---|---|
| 💬 Chat en vivo | Lee el chat de Kick y responde a comandos |
| 🎬 Cola de videos | El chat pide videos de YouTube con `!play` |
| 🔊 Text-to-Speech | El chat hace hablar al bot con `!s` y otras voces |
| 🎮 Entretenimiento | Dados, 8-ball y sorteos con `!dado`, `!8ball`, `!sorteo` |
| 📢 Respuestas automáticas | Discord, PC, horario, seguidores, uptime… |
| 🛡️ Anti-spam | Cooldowns por usuario y globales en todos los comandos |
| 📺 Overlay OBS | Pantalla animada estilo pixel art con stats y reproductor |
| 👥 Meta de seguidores | Barra de progreso en tiempo real vía Pusher |
| 💻 Stats del sistema | CPU, RAM y temperatura en el overlay |

---

## Instalación (usuarios finales)

1. Descarga `DaiBot.exe` desde Releases
2. Si aparece la pantalla azul **"Windows protegió su PC"**:
   - Haz clic en **"Más información"**
   - Luego en **"Ejecutar de todas formas"**
   > Esto ocurre porque el instalador es nuevo y Windows aún no lo reconoce. No es un virus.
3. Ejecútalo — no requiere permisos de administrador
4. Al terminar, haz doble clic en el icono del Escritorio
5. Se abrirá tu navegador para iniciar sesión en Kick
6. Listo — el bot se configura solo

**No necesitas instalar** Rust, Python, Node.js ni nada adicional. Todo viene incluido en el instalador.

> El overlay principal es `http://localhost:3000/pixel.html` — agrégalo como Browser Source en OBS.

---

## Configurar OBS

Agrega una **Browser Source** con estos ajustes:

| Campo | Valor |
|---|---|
| URL | `http://localhost:3000/pixel.html` |
| Ancho | `1920` |
| Alto | `1080` |
| Controlar audio vía OBS | ✅ Marcado |

> ⚠️ Usa **una sola** browser source. Refrescar crea una segunda conexión.

El overlay se personaliza solo con tu nombre de canal al conectarse.

---

## Comandos del chat

![Comandos](comandos.png)

### Cualquiera del chat

| Comando | Descripción | Cooldown |
|---|---|---|
| `!play [url]` | Agrega un video de YouTube a la cola | 30s por usuario |
| `!cola` | Muestra los próximos videos en cola | 30s global |
| `!quitarme` | Elimina tu primer video de la cola | — |
| `!misongs` | Ve tus videos con su posición en cola | 15s por usuario |
| `!s [texto]` | TTS voz Camila (acento peruano) | 15s por usuario |
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

### Requisitos

- [Rust](https://rustup.rs) (toolchain stable)
- [Inno Setup 6](https://jrsoftware.org/isdl.php) (para compilar el instalador)

### Compilar el instalador

```powershell
cd installer
.\build.ps1
```

`build.ps1` hace todo automáticamente:
1. `cargo build --release` — compila el bot
2. Genera `icon.ico` desde `icon_source.png` si existe
3. Descarga Python 3.12 embeddable + instala edge-tts (una sola vez, queda en caché)
4. Compila el instalador con Inno Setup → `installer/output/DaiBot.exe`

### Credenciales OAuth

Antes de compilar, edita `installer/DaiBot.iss` y pon tus credenciales de la app de Kick:

```
#define KickClientId     "tu_client_id"
#define KickClientSecret "tu_client_secret"
```

Crea la app en [kick.com/settings/developer](https://kick.com/settings/developer) con redirect URL `http://localhost:3001/callback`.

### Icono personalizado

Coloca tu imagen como `installer/icon_source.png` y `build.ps1` la convertirá a `.ico` automáticamente.

### Tests

```powershell
cd backend
cargo test
```

43 tests unitarios: cooldowns, cola de videos, voces TTS, parsing de URLs de YouTube y helpers de configuración.

---

## Estructura del proyecto

```
DaiBotkick/
├── .env.example            ← Plantilla de configuración
├── comandos.html           ← Fuente del diseño de la imagen de comandos
├── comandos.png            ← Imagen de comandos lista para el stream (4K)
│
├── backend/                ← Servidor en Rust (axum + socketioxide)
│   └── src/
│       ├── main.rs         ← Punto de entrada, AppState, primer arranque
│       ├── login.rs        ← OAuth 2.0 PKCE en Rust (sin Node.js)
│       ├── config.rs       ← Carga de variables de entorno
│       ├── commands/       ← Lógica de todos los comandos del chat
│       ├── cooldown.rs     ← Anti-spam: cooldowns por usuario y globales
│       ├── kick/           ← Conexión al chat de Kick.com (Pusher WebSocket)
│       ├── tts/            ← Text-to-speech vía edge-tts bundled
│       ├── queue/          ← Cola de videos con persistencia en disco
│       ├── server/         ← WebSocket con el overlay (Socket.IO)
│       └── stats/          ← CPU/RAM en tiempo real
│
├── overlay/                ← Archivos servidos al OBS
│   └── pixel.html          ← Overlay principal (pixel art, chat, reproductor)
│
├── login/                  ← Login OAuth legacy (Node.js, referencia)
│   └── login.js
│
├── installer/              ← Empaquetado para Windows
│   ├── DaiBot.iss          ← Script de Inno Setup 6
│   ├── build.ps1           ← Compila todo: Rust + icono + Python + instalador
│   ├── make_icon.ps1       ← Convierte icon_source.png → icon.ico
│   └── icon_source.png     ← Imagen fuente del icono (no en git si es privada)
│
└── data/                   ← Datos en tiempo real (no en git)
    └── tts_cache/          ← Cache de audios MP3 generados
```

---

## Tecnologías

- **Backend:** Rust — [Axum](https://github.com/tokio-rs/axum), [socketioxide](https://github.com/Totodore/socketioxide), tokio, reqwest
- **Overlay:** HTML + CSS + JavaScript vanilla
- **Chat:** Kick.com Public API v1 (OAuth 2.0 PKCE) + Pusher WebSocket
- **TTS:** [edge-tts](https://github.com/rany2/edge-tts) — voces de Microsoft, incluido en el instalador
- **Videos:** YouTube IFrame con autoplay y unmute vía postMessage
- **Instalador:** [Inno Setup 6](https://jrsoftware.org/isdl.php) con Python 3.12 embeddable bundled

---

## Solución de problemas

**El overlay no se ve en OBS**
→ Verifica que DaiBot esté corriendo antes de abrir OBS
→ URL correcta: `http://localhost:3000/pixel.html`

**El bot no se conecta al chat**
→ Usa el acceso directo "Configurar OAuth" del Menú Inicio para renovar el token

**¿Dónde está el panel de control?**
→ Aún no existe — el bot se controla desde los comandos del chat

**No se escucha el TTS**
→ Verifica que "Controlar audio vía OBS" esté marcado en la browser source

**El video no reproduce**
→ Asegúrate de tener una sola browser source en OBS

**El overlay se ve cortado**
→ El overlay está diseñado para 1920×1080. Verifica las dimensiones en OBS
