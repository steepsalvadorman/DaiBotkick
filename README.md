# DaiBot para Kick

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

## Comandos

| Comando | Función |
|---|---|
| `!play URL` | Agregar un video de YouTube; 30 s por usuario |
| `!cola` | Ver próximos videos; 30 s global |
| `!quitarme` | Quitar tu primer video |
| `!misongs` | Ver tus videos; 15 s por usuario |
| `!dai TEXTO`, `!dalia TEXTO`, `!jorge TEXTO`, `!alex TEXTO` | TTS; cooldown compartido de 15 s |
| `!dado`, `!8ball PREGUNTA` | Entretenimiento |
| `!participar` / `!sorteo` | Entrar al sorteo abierto |
| `!uptime` | Tiempo desde el inicio real del stream, si está disponible |
| `!seguidores` | Total y meta, si Kick proporciona ese dato |
| `!discord`, `!redes`, `!pc`, `!horario`, `!comandos` | Información |

Solo la identidad del broadcaster, verificada por su ID en el webhook firmado, puede usar `!von`, `!voff`, `!vstop`, `!skip` / `!next` y `!sorteo abrir|cerrar|ganador`. El skip modifica la cola en el backend incluso sin overlay conectado.

Las respuestas y la meta son campos por canal en PostgreSQL: `cmd_discord`, `cmd_redes`, `cmd_pc`, `cmd_horario`, `follow_goal`. Una reautorización conserva esos valores. No se leen de variables `CMD_*`.

Se aceptan videos directos MP4/WebM/MOV/M4V por HTTPS, con validación de URL y rechazo de direcciones locales literales. Un host DNS externo puede resolver o redirigir a otra dirección; usa enlaces de proveedores de confianza. La cola admite 100 elementos. TTS admite 350 caracteres, 32 pendientes por canal y dos síntesis simultáneas en el proceso. La síntesis tiene timeout de 30 s; la caché conserva hasta aproximadamente 256 archivos durante 24 h y puede regenerarse.

## Desarrollo local

Requisitos: Rust 1.88.0, PostgreSQL y `edge-tts==7.2.8` en el PATH. Node 22 o posterior se usa para las pruebas del overlay.

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

CI ejecuta formato, Clippy, tests Rust, integración PostgreSQL, backup/restauración, tests del overlay y build Docker en Linux. Arranca además la imagen y comprueba usuario sin privilegios, paquete TTS, health checks, métricas y archivos publicados con `node --test tests/runtime.test.cjs`. Esa prueba también puede ejecutarse contra un backend de pruebas ya iniciado configurando `DAIBOT_BASE_URL`; la prueba de empaquetado espera `/comandos.html`, que el Dockerfile copia al directorio del overlay.

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
