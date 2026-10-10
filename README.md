# CuchurruminRix para Kick

Bot para varios canales de Kick, con comandos de chat, cola de YouTube/video, TTS, sorteos y overlay para OBS.

## Funcionamiento

```text
Kick -> webhook firmado -> inbox PostgreSQL -> comandos del canal
                                           -> Socket.IO -> overlay OBS
OAuth Kick -> PostgreSQL -> estado y tareas del canal
```

El backend usa Rust, Axum, socketioxide y sqlx. `pixel.html` carga `app.js` y `style.css`; esos son los archivos que debes editar para cambiar el overlay. El reproductor utiliza la [API oficial de YouTube](https://developers.google.com/youtube/iframe_api_reference).

EventSub es la única entrada de chat y alertas. Se verifica la [firma RSA de Kick](https://github.com/KickEngineering/KickDevDocs/blob/main/events/webhook-security.md), el timestamp y el ID de entrega. Pusher ya no forma parte del servidor activo.

## Conectar un canal y OBS

1. Abre la raíz del servicio y pulsa **Conectar con Kick**.
2. Autoriza la app y copia la URL completa de la pantalla de éxito, incluido `#token=...`.
3. En OBS agrega una Browser Source de 1920×1080 y activa **Controlar audio vía OBS**.

La URL privada tiene este formato:

```text
https://tu-servicio/pixel.html?ch=tu_canal#token=TOKEN_GENERADO
```

Conserva esa URL en OBS. El token autoriza solo reproducción, no comandos de panel, y se envía en el handshake de Socket.IO. Un único socket autorizado controla video y TTS por canal. Una segunda fuente espera a que la primera se desconecte.

La URL sin token muestra chat y estadísticas, pero no reproduce audio/video ni puede avanzar la cola. Los canales registrados antes de esta actualización deben volver a conectarse por OAuth para obtener su URL privada. `index.html` conserva canal y token al redirigir.

Si el navegador bloquea autoplay, interactúa con la fuente de OBS. La pantalla muestra el estado de conexión y errores de reproducción. Un video tiene un límite de reproducción de diez minutos.

La topbar del overlay queda a 4 px del borde superior del lienzo y muestra el nombre del canal sin etiquetas de sistema. El panel `music` se despliega desde la barra cuando el reproductor autorizado tiene contenido visible y se repliega al vaciar la cola, ocultarlo o desconectarse. La transición se desactiva si el navegador solicita movimiento reducido.

Las esquinas inferiores llevan dos discos vintage azul Aero parcialmente recortados, limitados a los últimos 42 px del lienzo para dejar libre el chat. Son decorativos, estáticos y no interceptan clics.

El encabezado del chat, los comentarios y las recomendaciones rotativas comparten el acabado Aero azul oscuro de la topbar: cristal pulido, bordes luminosos y texto claro. Conservan su posición por encima de los discos inferiores.

El chat conserva emojis Unicode y convierte los emotes de Kick (`[emote:ID:NOMBRE]`) en imágenes del CDN oficial, incluidos emotes animados. Si una imagen falla, muestra su nombre y un aviso. El resto del mensaje se renderiza como texto, nunca como HTML.

La cápsula Aero de emojis muestra el total y los tres más usados en una ventana móvil de cinco minutos (se actualiza cada cinco segundos). Cuenta emotes de Kick y emojis Unicode completos, sin separar familias, banderas o tonos de piel. Es local a la fuente: recargar reinicia el contador y no recupera mensajes perdidos durante desconexiones.

Para usar el mouse, haz clic derecho en la fuente de navegador de OBS y selecciona **Interactuar**. Los botones de la topbar permiten mostrar/ocultar el reproductor sin parar el audio, alternar el chat (también con `?chat=0`), consultar la cola y ajustar el volumen de música (no el TTS). Escape cierra los menús. Estas preferencias son locales a esa fuente y se restablecen al recargar; no otorgan permisos de panel. La imagen compuesta de OBS no es una ventana interactiva encima del juego: los clics del juego no llegan al overlay.

La fuente autorizada también puede reiniciar la canción actual, pausar/reanudar y pasar a la siguiente mediante el avance validado por ID y versión. El vinilo y las barras de la topbar se animan durante la reproducción (son decorativos, no un analizador de audio). Reiniciar conserva la pausa; el límite de diez minutos por video sigue contando durante las pausas. Una vista pública no puede usar estos controles.

## Comandos

| Comando | Función |
|---|---|
| `!play URL` / `!play nombre y artista` | Agregar un video o buscar una canción en YouTube; 30 s por usuario |
| `!cola` | Ver próximos videos; 30 s global |
| `!quitarme` | Quitar tu primer video |
| `!misongs` | Ver tus videos; 15 s por usuario |
| `!voces` | Listar las voces y cómo usarlas; 20 s global. Incluye `!jacinta` si Fish Audio tiene clave configurada |
| `!dai TEXTO`, `!dalia TEXTO`, `!jorge TEXTO`, `!alex TEXTO` | Voces TTS oficiales; cooldown compartido de 15 s |
| `!narrador TEXTO`, `!epico TEXTO`, `!comedia TEXTO` | Voces TTS oficiales con prosodia genérica; «usuario dice: texto» |
| `!jacinta TEXTO` | Voz autorizada de Fish Audio; requiere `FISH_AUDIO_API_KEY` |
| `!dado`, `!8ball PREGUNTA` | Entretenimiento |
| `!ruleta` / `!bingo abrir` | Dueño: abrir un bingo de 75 bolas e inscripciones |
| `!carton` | Registrar un cartón 5×5 y recibir el enlace personal para verlo y marcarlo; repetir devuelve el mismo enlace |
| `!bingo iniciar` / `!bingo cancelar` | Dueño: cerrar inscripciones e iniciar bolas cada 10 s, o cancelar |
| `!bingo` | Reclamar victoria: el servidor comprueba fila, columna o diagonal del cartón registrado |
| `!participar` / `!sorteo` | Entrar al sorteo abierto |
| `!uptime` | Tiempo desde el inicio real del stream, si está disponible |
| `!seguidores` | Total y meta, si Kick proporciona ese dato |
| `!discord`, `!redes`, `!pc`, `!horario`, `!comandos` | Información |

Solo la identidad del broadcaster, verificada por su ID en el webhook firmado, puede usar `!von`, `!voff`, `!vstop`, `!skip` / `!next` y `!sorteo abrir|cerrar|ganador`. El skip modifica la cola en el backend incluso sin overlay conectado.

### Bingo Aero

El dueño abre con `!bingo abrir` (o `!ruleta`). Los participantes escriben `!carton` antes del inicio: el bot responde con un enlace personal a una página Aero con su cartón 5×5. El chat no abre pestañas automáticamente; cada participante pulsa su enlace. La página permite marcar/desmarcar números, guarda las marcas en ese navegador si el almacenamiento está disponible y señala las bolas sorteadas consultando el servidor cada 3 segundos. `LIBRE` ya cuenta. Repetir `!carton` devuelve el mismo enlace incluso durante la partida.

El enlace se publica en el chat, pero **solo su dueño puede ver el cartón tras iniciar sesión con Kick**. El backend verifica el nombre de la cuenta autorizada mediante `/users` de Kick y lo compara con quien registró el cartón; un enlace copiado no basta. Este OAuth de espectadores solo solicita `user:read`: no registra canales, no cambia el stream ni conserva tokens de acceso de Kick. La sesión usa una cookie HttpOnly, SameSite=Lax y Secure con HTTPS, dura 8 horas y se pierde al reiniciar el backend. El estado OAuth es de un solo uso, está ligado al navegador, utiliza PKCE y caduca a los 10 minutos.

**Configuración de Kick:** añade `https://TU-DOMINIO/auth/bingo/callback` a las URLs de retorno autorizadas de la misma aplicación (además de `/auth/callback`, usado para conectar el canal). Configura `BASE_URL` con la URL pública HTTPS del backend para que los espectadores puedan abrir sus enlaces; `localhost` solo sirve en tu computadora. El token aleatorio del cartón queda invalidado al cancelar, abrir otra partida, reiniciar el backend o reconectar el canal. Se envía en el fragmento del enlace y en un encabezado al consultar `/api/bingo/:slug`, no en parámetros de la URL de la API. Las marcas siguen siendo locales al navegador, no se sincronizan entre dispositivos.

Las marcas manuales son una ayuda visual y no modifican el cartón ni las bolas sorteadas. La página indica si las bolas reales completan una línea, pero el participante debe escribir `!bingo` en el chat para reclamar y el servidor vuelve a verificarlo. Las columnas usan los rangos 1–15, 16–30, 31–45, 46–60 y 61–75. Hay un único cartón por usuario, hasta 1000 participantes, y un cooldown compartido de 5 s para sus comandos de bingo.

`!bingo iniciar` cierra inscripciones y saca una bola cada 10 segundos sin repetir, incluso sin OBS conectado. El overlay muestra un bombo de cristal, la última bola, cinco resultados recientes y las 75 casillas iluminadas; el botón `bingo` de la topbar permite ocultarlo localmente. Al reconectar recibe el estado completo.

Para ganar se escribe **`!bingo`**, sin números ni nombre ajeno. El servidor verifica el cartón del autor contra las bolas realmente sorteadas y acepta únicamente la primera reclamación válida de fila, columna o diagonal. Detiene el sorteo, anuncia al ganador en chat y muestra su cartón con la línea ganadora resaltada. Una reclamación falsa explica el rechazo y no cambia la partida. Tras las 75 bolas aún se puede reclamar.

`!bingo cancelar` limpia la partida; tras una victoria se puede abrir otra. Las partidas son temporales y por canal: **reiniciar el backend o reconectar la cuenta de Kick cancela la partida y sus cartones**. No se guardan en PostgreSQL ni otorgan premios o sanciones automáticamente.

Las respuestas y la meta son campos por canal en PostgreSQL: `cmd_discord`, `cmd_redes`, `cmd_pc`, `cmd_horario`, `follow_goal`. Una reautorización conserva esos valores. No se leen de variables `CMD_*`.

Se aceptan videos directos MP4/WebM/MOV/M4V por HTTPS, con validación de URL y rechazo de direcciones locales literales. Un host DNS externo puede resolver o redirigir a otra dirección; usa enlaces de proveedores de confianza. La cola admite 100 elementos. TTS admite 350 caracteres, 32 pendientes por canal y dos síntesis simultáneas en el proceso. La síntesis tiene timeout de 30 s; la caché conserva hasta aproximadamente 256 archivos durante 24 h y puede regenerarse.

Todos los comandos de voz del chat (incluidos `!s` y `!camila`) anuncian al autor con «usuario dice: mensaje» en la voz elegida. `!narrador`, `!epico` y `!comedia` son perfiles genéricos originales que ajustan voces oficiales de Edge TTS. `!jacinta` usa el modelo autorizado de Fish Audio configurado para esta voz y requiere la variable secreta `FISH_AUDIO_API_KEY`; el ID del modelo se incluye en el backend y la clave nunca debe guardarse en el repositorio. Para pronunciar el nombre, se utilizan hasta 40 caracteres y se sustituyen separadores y símbolos por espacios. El prefijo no consume los 350 caracteres del mensaje. Las alertas automáticas y el TTS del panel conservan su texto original.

Fish Audio utiliza `s2.1-pro-free` con `latency: "low"` para priorizar una menor demora de generación (puede reducir la calidad), sin acelerar la voz: velocidad `1.0`, volumen sin aumento y normalización de volumen activada. Los ajustes del sitio web de Fish Audio no se aplican automáticamente a las solicitudes del bot. El overlay espera el MP3 completo; todavía puede haber demora por generación, red y mensajes pendientes. La caché de Fish Audio incluye el modelo y los parámetros de síntesis para no reutilizar audios de configuraciones anteriores.

## Desarrollo local

Requisitos: Rust 1.88.0, PostgreSQL, `edge-tts==7.2.8` y `yt-dlp==2026.8.19` en el PATH. Node 22 o posterior se usa para las pruebas del overlay.

`!play cervecita flor pileña` busca los primeros cinco resultados de YouTube mediante `yt-dlp`, sin clave API ni descargar audio. Selecciona el primero en el orden de relevancia de YouTube que tenga duración conocida de hasta diez minutos y no sea un directo o estreno pendiente. La reproducción sigue usando el reproductor de YouTube y la cola habitual. No garantiza versiones exactas ni disponibilidad para incrustar: añade el artista o usa el enlace exacto. Hay dos búsquedas simultáneas como máximo y timeout de 25 s; los errores y bloqueos de YouTube se avisan en chat.

En Windows puedes usar el ejecutable oficial `yt-dlp.exe`; se detecta también en `%LOCALAPPDATA%\DaiBot\tools\yt-dlp.exe`. Configura `YT_DLP_PATH` con su ruta absoluta para otra ubicación si no está en el PATH. En servidores con Python se puede instalar `yt-dlp==2026.8.19` en el entorno del backend. La imagen de despliegue ya incluye esa dependencia.

Desde la raíz del repositorio, copia `.env.example` a `.env` y configura:

| Variable | Uso |
|---|---|
| `KICK_CLIENT_ID`, `KICK_CLIENT_SECRET` | Credenciales de la app OAuth |
| `DATABASE_URL` | PostgreSQL; desarrollo y producción deben usar bases diferentes |
| `BASE_URL` | Origen público del servidor; si se omite usa `RENDER_EXTERNAL_URL` o localhost |
| `PORT` | Puerto HTTP/Socket.IO; por defecto 3000 |
| `OVERLAY_DIR` | Ruta de archivos del overlay; por defecto `../overlay` |
| `TTS_CACHE_DIR` | Directorio temporal de caché; opcional |
| `YOUTUBE_API_KEY` | Opcional, para el primer elemento de playlists; no hace falta para un video individual |
| `RUST_LOG` | Nivel de logs; por defecto info |

No hay claves de YouTube incluidas en el código. Si utilizabas la clave de la versión anterior, revisa restricciones y rotación en Google Cloud antes de habilitar playlists.

```powershell
cd backend
cargo run --locked
```

Registra en la app de Kick `BASE_URL/auth/callback` como redirect y `BASE_URL/kick_webhook` como webhook. Para recibir webhooks locales necesitas una URL HTTPS pública mediante un túnel. El flujo de registro es `/auth/kick`; el modo antiguo `--login` solo guarda tokens en `.env` y no registra canales del servidor actual.

Las migraciones versionadas se ejecutan al arrancar. Si falla una migración o la carga de canales, el servidor termina con error. Las tablas de una instalación anterior creadas sin sqlx se conservan mediante `CREATE TABLE IF NOT EXISTS`; respalda la base antes de actualizar.

## Verificación

```powershell
cargo +1.88.0 fmt --manifest-path backend/Cargo.toml --all -- --check
cargo +1.88.0 clippy --manifest-path backend/Cargo.toml --locked --all-targets -- -D warnings
cargo +1.88.0 test --manifest-path backend/Cargo.toml --locked
node --test tests/overlay.test.cjs
```

Las ocho pruebas de integración requieren PostgreSQL de pruebas y permiso para crear/eliminar schemas aislados. Incluyen OAuth/Kick simulados por HTTP, conexiones Socket.IO reales, aislamiento, renovación concurrente, fallos de DB y recuperación del inbox. Nunca las apuntes a producción:

```powershell
$env:TEST_DATABASE_URL = 'postgresql://postgres:postgres@localhost:5432/daibot_test'
cargo +1.88.0 test --manifest-path backend/Cargo.toml --locked -- --ignored
```

Para comprobar backup/restauración, instala `psql`, `pg_dump`, `pg_restore` y Python 3, configura `TEST_DATABASE_URL` y ejecuta `bash tests/backup_restore.sh`. Crea fixtures en un schema y una base desechables, comprueba configuración, tokens, cola e inbox restaurados y limpia sus datos. Requiere permiso para crear/eliminar bases; no prueba un backup de producción ni un rollback de imagen.

Antes de desplegar, valida en local desde `backend/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` y, con `TEST_DATABASE_URL` apuntando a una base de pruebas, `cargo test -- --ignored`. Los tests del overlay se ejecutan con `node --test tests/overlay.test.cjs`. `tests/runtime.test.cjs` comprueba un backend ya iniciado (configura `DAIBOT_BASE_URL`); espera `/comandos.html`, que el Dockerfile copia al directorio del overlay.

La validación local del 3 de octubre de 2026 pasó con Rust 1.88.0 y Node 22.16.0 en Linux: 43 tests aislados, ocho integraciones con PostgreSQL 18.6, doce tests del overlay, cuatro del backend en ejecución, formato, Clippy y build release. Pasó el backup/restauración de fixtures y `/readyz` pasó de 200 a 503 al detener la DB y volvió a 200 al reiniciarla. CI usa PostgreSQL 16; su ejecución y la imagen Docker siguen pendientes porque esta sesión no tiene acceso al servicio Docker.

Las pruebas del overlay simulan DOM y media: no sustituyen una sesión real de OBS/YouTube. Consulta [el plan de pruebas](PLAN_PRUEBAS_DESPLIEGUE.md) para la validación real y de carga.

## Despliegue

`Dockerfile` compila con Rust 1.88.0 y `--locked`, instala TTS en un entorno Python y ejecuta el servicio sin privilegios. `.dockerignore` excluye credenciales y builds locales.

### Railway (producción)

`railway.json` construye con el `Dockerfile`, comprueba `/readyz` y reinicia el servicio si falla. En el proyecto de Railway:

1. *New Project → Deploy from GitHub repo* con este repositorio.
2. *+ New → Database → PostgreSQL* en el mismo proyecto.
3. Variables del servicio del bot: `DATABASE_URL=${{Postgres.DATABASE_URL}}`, `KICK_CLIENT_ID`, `KICK_CLIENT_SECRET` y `BASE_URL` con el dominio público (sin `/` final). `PORT` lo asigna Railway.
4. *Settings → Networking → Custom Domain*: añade el dominio y crea en tu DNS el CNAME que indique Railway (en Cloudflare, con el proxy desactivado hasta que Railway emita el certificado).
5. En la app de Kick: Redirect URL `<BASE_URL>/auth/callback` y webhook `<BASE_URL>/kick_webhook`.

Mantén una sola réplica.

### Render

`render.yaml` usa una instancia de web service pago y recibe `DATABASE_URL` como secreto para una base persistente, por ejemplo Supabase o PostgreSQL administrado pago. Ya no crea una base gratuita. Los servicios gratuitos y la base Free tienen límites que debes revisar en la [documentación de Render](https://render.com/docs/free).

Configura credenciales, base y `BASE_URL` en Render; registra ese origen en Kick y lanza staging primero. Los despliegues automáticos esperan a los checks de CI mediante [`autoDeployTrigger: checksPass`](https://render.com/docs/blueprint-spec). No se ha publicado ninguna versión desde esta revisión.

- `/healthz`: proceso HTTP activo.
- `/readyz`: proceso disponible y PostgreSQL responde; devuelve 503 si falla.
- `/metrics`: conteos de canales, sockets, TTS pendientes y webhooks recibidos/rechazados/duplicados/procesados. Los contadores se reinician al reiniciar el proceso.

Mantén una sola instancia. Durante un despliegue Render puede solapar instancias brevemente: las filas del inbox se bloquean con `FOR UPDATE SKIP LOCKED` y las escrituras de cola comprueban el estado anterior, pero Socket.IO y elección de reproductor siguen siendo locales. No hay soporte completo para varias réplicas. Para un stream crítico, actualiza en una ventana de mantenimiento hasta contar con coordinación y distribución de eventos entre procesos.

## Persistencia y recuperación

La cola, su versión, los IDs de elementos, visibilidad del video, configuración y tokens se guardan en PostgreSQL. Un reinicio recupera el primer video desde el comienzo; no guarda el segundo exacto de reproducción. Sorteos, cooldowns y TTS pendientes se reinician.

El webhook responde después de guardar el evento en un inbox. Reentregas con el mismo ID no insertan otro evento. La ejecución tiene semántica de **al menos una vez**: un crash después de enviar audio/chat y antes de confirmar la fila puede repetir un efecto. No se garantiza ejecución exactamente una vez de servicios externos. Los eventos procesados se limpian después de 24 h; los pendientes permanecen para recuperación.

Reautorizar cancela las tareas del estado anterior, preserva configuración y cola, y reconecta sockets. El apagado atiende Ctrl+C/SIGTERM y cancela tareas; los subprocess TTS se terminan al cancelar. Las respuestas 401 concurrentes del token anterior comparten una renovación. Si falla PostgreSQL justo después de que Kick rota un refresh token, el proceso conserva el token nuevo en memoria y recuerda el último valor persistido para recuperar la escritura en la siguiente renovación. Un reinicio antes de persistirlo todavía puede requerir reconectar el canal.

Para rollback, conserva la imagen/commit anterior y un backup probado. Volver a una imagen no revierte migraciones ni efectos externos. Esta versión cambia el protocolo de cola (`items`, `version`, ID único) y autorización del overlay: despliega backend y overlay juntos, y actualiza la URL de OBS al migrar.
