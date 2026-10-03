# DaiBot: revisión, pruebas y despliegue

Revisión del código disponible el 3 de octubre de 2026. El diagnóstico siguiente conserva la situación encontrada antes de los cambios; consulta el estado actualizado aquí y el README para operar la versión nueva.

## Estado de implementación

Aplicado: firma RSA y timestamp en webhooks; inbox persistente con deduplicación; EventSub como entrada única; identidad del broadcaster por ID; texto externo sin `innerHTML`; cola persistente/versionada con IDs únicos; un reproductor autenticado por canal con token separado del panel; skip en backend; cancelación al reautorizar/apagar; OAuth con cookie y consumo atómico de state; errores DB visibles y migraciones sqlx; eliminación de clave YouTube del código; límites/timeout/caché TTS y audio secuencial; cooldown atómico; estadísticas sin confundir suscriptores y seguidores; extracción de JS/CSS a app.js/style.css; redirección conservando query/token; health checks, métricas, Docker sin privilegios, plantilla vigente y CI.

Verificado localmente el 3 de octubre de 2026: formato, Clippy sin warnings, 43 tests Rust aislados y build release con Rust 1.88.0 en Linux; ocho integraciones con PostgreSQL temporal 18.6; doce pruebas del overlay y cuatro del backend en ejecución con Node 22.16.0 en Linux. También pasó backup/restauración de fixtures y recuperación de readiness al detener/reiniciar PostgreSQL. Docker está instalado, pero la sesión no tiene permisos sobre su socket; no se ejecutó la imagen. CI usa PostgreSQL 16 y ahora incluye backup/restauración y arranque/verificación de la imagen. No se ha verificado una ejecución remota de CI desde esta sesión.

Pendiente de validación real: OAuth y eventos de Kick, autoplay/audio en OBS, playlists con una clave propia, interrupciones de servicios externos y DB en staging, carga prolongada de varios canales, backup operativo y rollback de imagen. El inbox admite reejecución tras un crash: puede repetir efectos externos ya enviados; ese caso ahora tiene una prueba de integración. La cola recupera el video desde el comienzo; sorteos, cooldowns y TTS pendientes quedan en memoria. Varias réplicas requieren coordinación adicional; hacer el primer rollout en staging y una ventana de mantenimiento.

### Seguimiento de tareas pendientes

- [x] Validar formato, Clippy, tests y compilación release en Linux con el toolchain del Dockerfile.
- [x] Ejecutar integración PostgreSQL y corregir su fixture Socket.IO, que no registraba el namespace antes de emitir eventos.
- [x] Probar OAuth/PKCE con servicio HTTP simulado, cookie incorrecta, callbacks concurrentes, rechazos del proveedor e identidad inválida.
- [x] Probar reautorización: cancelar estado antiguo y preservar cola, visibilidad y token de reproducción.
- [x] Probar conexiones Socket.IO reales: tokens público/incorrecto/privado, panel separado, dos ACK simultáneos, takeover y aislamiento de dos canales.
- [x] Probar identidad firmada: un viewer con nombre del streamer no puede saltar la cola; el broadcaster sí.
- [x] Probar refresh concurrente, `expires_in` de 90 s, 401 tardíos y persistencia después de una escritura rechazada temporalmente.
- [x] Corregir recuperación de refresh tokens: recordar el valor de DB permite guardar la siguiente rotación sin restaurar el token antiguo; los 401 del token anterior comparten refresh.
- [x] Probar inbox con efecto real aislado por canal, reentrega y reejecución tras interrumpir una respuesta externa antes del commit.
- [x] Probar cola con conflicto entre estados, error DB sin cambio en memoria y readiness negativa.
- [x] Ampliar pruebas del overlay: redirección, autoplay bloqueado, pérdida del rol, reconexión, ACK perdido y callbacks de videos antiguos.
- [x] Corregir reintento de la API de YouTube tras un fallo de red; el siguiente video puede volver a cargar el script.
- [x] Ejecutar `tests/backup_restore.sh`: restaurar configuración, tokens, visibilidad, IDs/versiones de cola e inbox pendiente en una base desechable. No demuestra restauración de un backup operativo ni rollback del backend.
- [x] Arrancar binario release en Linux con DB de pruebas y archivos preparados como en la imagen: health checks, métricas y recursos pasan las cuatro pruebas de `tests/runtime.test.cjs`.
- [x] Detener/reiniciar la DB temporal: `/healthz` permanece en 200, `/readyz` devuelve 503 y recupera 200 en 0,02 s desde la primera consulta tras reiniciar. Es una comprobación local, no una medición de staging.
- [x] Añadir a CI backup/restauración y arranque de la imagen para verificar usuario 10001, paquete TTS, rutas y readiness.
- [ ] Ejecutar y revisar CI con PostgreSQL 16 y build/arranque Docker; no hay acceso al daemon en esta sesión.
- [ ] Validar síntesis real/TTS, OAuth/EventSub reales, OBS/YouTube y playlists con clave propia en staging.
- [ ] Medir carga: 10 canales × 2 sockets, pico de 20 mensajes/s, p95 y sesión de cuatro horas; validar crecimiento de RAM/disco y proveedores caídos.
- [ ] Restaurar un backup operativo y probar redeploy/rollback de imagen en staging.
- [ ] Lanzar canal piloto y supervisar un stream completo antes de ampliar.

Los apartados de diagnóstico y verificaciones originales siguientes describen el estado anterior a los cambios; las casillas anteriores registran la validación actual. Las pruebas automatizadas utilizan credenciales y eventos sintéticos, sin llamar a Kick real.

## Arquitectura y alcance

- Rust: Axum sirve OAuth, webhooks, archivos estáticos y Socket.IO; sqlx conecta PostgreSQL.
- Cada canal tiene cola de videos, cooldowns, sorteo, tokens, tareas de Kick, estadísticas y TTS.
- El overlay usado en OBS es `overlay/pixel.html`, con CSS y JavaScript internos. No carga `app.js` ni `style.css`.
- `overlay/app.js` conserva otra implementación: espera un array en `syncQueue`, mientras el servidor envía `{items: [...]}`. `index.html` redirige a `pixel.html` y pierde el query `?ch=`.
- `sio.js` es el cliente Socket.IO distribuido. `comandos.html` y `comandos.png` son material auxiliar; el Dockerfile solo copia `overlay/`, por lo que esos archivos no se publican.
- `config.rs` y el login local conservan lógica del modo anterior. El servidor actual registra canales mediante OAuth y PostgreSQL; los tokens de `.env` del login local no registran un canal en esa base.
- Se revisaron los módulos del backend, migración, overlay, Dockerfile, Blueprint, plantilla de variables y README. No se ha verificado una sesión real de Kick ni OBS.

## Hallazgos prioritarios

| Prioridad | Evidencia | Consecuencia y acción propuesta |
|---|---|---|
| P0 | `main.rs`, handler `kick_webhook`: no valida firma ni identifica entregas repetidas | Un POST puede simular chat y nombre del streamer, activar comandos exclusivos y producir TTS. Validar firma sobre los bytes originales, timestamp y deduplicar por ID antes de ejecutar efectos. |
| P0 | `pixel.html`: chat y títulos se interpolan en `innerHTML`; también sucede en `app.js` | Riesgo de ejecución de HTML/script desde datos externos. Crear elementos y usar `textContent`; verificar con entradas adversas en staging. |
| P0 | `queue/mod.rs`, tests: usan `fs::write` sin importar `fs` y llaman `VideoQueue::load`, que ya no existe | La suite quedó ligada a persistencia eliminada. Adaptarla al contrato actual o implementar la persistencia acordada; no borrar pruebas para ocultar la regresión. |
| P1 | `channel.rs`: elimina la entrada de DashMap, pero no cancela tareas ni reemplaza handlers de sockets existentes | Reautorizar un canal puede dejar lectores, polling y refresh antiguos activos, y sockets apuntando al estado anterior. Gestionar cancelación y ciclo de vida; reconectar/sincronizar clientes. |
| P1 | Webhook y Pusher procesan ambos chat y alertas; no hay deduplicación común | Un evento puede generar chat o alertas duplicadas. Preferir EventSub como entrada principal y definir cuándo opera el fallback. |
| P1 | `server/mod.rs`: `advanceQueue` permite escribir a cualquier socket del canal | Quien conoce el slug y el video actual puede saltarlo. Separar espectadores de un reproductor autorizado. Usar ID único por elemento y versión de cola para evitar dobles avances, incluso con dos videos iguales consecutivos. |
| P1 | `db.rs`: varios errores se ignoran; migración advierte y continúa | OAuth puede anunciar éxito aunque no se guardó el canal; un fallo de lectura aparenta cero canales. Devolver errores, migraciones versionadas y readiness real. |
| P1 | `commands/mod.rs`: clave de YouTube escrita en código | Retirar del código, pasar por variable secreta y revisar restricciones/rotación en Google Cloud si es una credencial propia. No reproducirla en logs o documentación. |
| P1 | `kick/sender.rs`: `&text[..text.len().min(60)]` | Un mensaje con tildes o emojis en el límite puede provocar panic por cortar UTF-8. Truncar por caracteres. |
| P1 | `state.rs`, `tts/mod.rs`, `edge_tts.rs` | Cola TTS sin límite, caché sin limpieza y subprocess sin timeout; el frontend reproduce cada audio inmediatamente, pudiendo superponer voces. Aplicar límites, timeout, TTL y reproducción secuencial. |
| P1 | Cola, sorteo, cooldowns y rooms viven en memoria | Reiniciar pierde estado. Varias réplicas no comparten cola ni eventos y pueden renovar el mismo token. Empezar con una instancia y definir persistencia antes de escalar. |
| P2 | `auth.rs` usa defaults `unknown`/`0`, estado OAuth no vinculado a sesión; `db.rs` hace SELECT y DELETE separados | Validar identidad y respuestas HTTP antes de guardar; vincular state al navegador y consumirlo atómicamente con DELETE RETURNING. Probar callbacks concurrentes y expirados. |
| P2 | Refresh fija siguiente vencimiento a 7200 segundos aunque la respuesta trae `expires_in` | Probar expiraciones distintas y refresh concurrente; centralizar renovación y usar el vencimiento real. |
| P2 | Cooldown se comprueba y registra con locks separados | Dos eventos concurrentes pueden superar el mismo cooldown. Hacer check y registro bajo un único lock; ejecutar limpieza periódica. |
| P2 | Estadísticas usan `subscriber_count` como fallback de seguidores; uptime mide tiempo desde inicio del canal/browser | Datos potencialmente engañosos. Validar esquema real y calcular uptime desde el inicio del stream. |
| P2 | `pixel.html`: cambio de cabeza no fuerza cambio de reproductor; `!skip` depende de un cliente conectado | Eliminar el video actual puede dejar el anterior reproduciéndose. El backend debe gobernar avances; comparar ID de reproducción en cada sincronización. |
| P2 | `main.rs` mata procesos que ocupen el puerto al iniciar en Windows | No ejecutar el backend contra un puerto compartido. Sustituir por error claro de puerto ocupado. |
| P2 | `.env.example` omite DATABASE_URL/BASE_URL; README anuncia 43 tests y pruebas de helpers que no están | Unificar documentación, variables vigentes y onboarding. Actualmente hay 33 funciones con `#[test]` en el código. |

## Preparación del entorno de pruebas

1. Crear staging con base independiente y una app OAuth de pruebas, si Kick permite registrarla. Usar dos canales de prueba A y B y una cuenta espectadora.
2. Registrar exactamente `https://<staging>/auth/callback` y `https://<staging>/kick_webhook`. Usar HTTPS público para pruebas integradas.
3. Configurar KICK_CLIENT_ID, KICK_CLIENT_SECRET, DATABASE_URL, BASE_URL, PORT, OVERLAY_DIR y TTS_CACHE_DIR. No reutilizar datos/tokens de producción.
4. Preparar fixtures basados en payloads oficiales de chat, follow, suscripción y regalos. Crear mocks para Kick/OAuth, noembed y síntesis TTS; hacer configurables los clientes/endpoints necesarios.
5. Registrar por caso: commit, entorno, entrada, resultado esperado, resultado observado y evidencia sin secretos. En OBS registrar versión, resolución y ajustes de audio.

## Plan de pruebas por fases

### Fase 1: compilación y pruebas aisladas

Ejecutar desde `backend/`:

```powershell
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

`clippy` y formato son controles propuestos; no se consideran ejecutados por aparecer aquí. Separar formato preexistente de defectos funcionales. Verificar también el build con la versión exacta de Rust del Dockerfile; pasar en Windows con otro toolchain no demuestra que el contenedor compile.

Cubrir lógica de cola y UUID por elemento, cooldown concurrente, aliases TTS, parsing estricto de URL/dominio, sorteos sin participantes y únicos, truncado Unicode y autorización por identidad validada. Las pruebas de cola deben reflejar explícitamente si habrá persistencia.

Salida: suite ejecutable, build reproducible y regresiones de P0/P1 con pruebas útiles.

### Fase 2: integración con PostgreSQL y servicios simulados

| Caso | Entrada / acción | Resultado esperado |
|---|---|---|
| OAuth válido | Autorizar canal nuevo | PKCE correcto, identidad válida, tokens guardados; una sola inicialización del canal. |
| OAuth inválido | Denegar, omitir code/state, state vencido o repetir callback | Error claro, sin alta ni tokens expuestos; state consumible una sola vez. |
| Callback concurrente | Dos callbacks con el mismo state | Solo uno consume state. |
| Reautorizar | Registrar A nuevamente | Configuración preservada, tareas antiguas detenidas, sockets sincronizados. |
| Fallo DB | Interrumpir escritura/migración/lectura | Error visible y readiness negativa donde corresponda; no anunciar éxito falso. |
| Renovación | 401, expires_in corto, refresh inválido y dos renovaciones simultáneas | Renovación única, persistencia consistente, recuperación o solicitud clara de reautorización. |
| Webhook auténtico | Firma válida y payload conocido | Un efecto en el canal correcto. |
| Webhook falso | Sin firma, firma errónea, cuerpo alterado, timestamp vencido | Rechazo sin efectos. |
| Reentrega | Mismo ID dos veces y por dos transportes | Un mensaje, una alerta, un comando. |
| Aislamiento | Comando/evento para A, overlays A y B conectados | Nada aparece ni cambia en B; tokens y permisos no se cruzan. |
| Panel | Token ausente/incorrecto/válido y canal con token vacío | Fallar cerrado; solo autorización válida permite cambios. |
| Avance | Socket público, reproductor autorizado, ID antiguo y dos ACK simultáneos | Solo el reproductor autorizado avanza una vez la versión esperada. |

Salida: integración reproducible y aislamiento entre canales demostrado.

### Fase 3: overlay y OBS de extremo a extremo

| Área | Escenarios | Criterio |
|---|---|---|
| Conexión | URL con/sin ch, canal inexistente, refresh, index.html?ch=A | Canal correcto o error visible; redirección conserva query; sin errores JS. |
| Cola | Agregar 3 videos, terminar, skip, quitar cabeza, clear, duplicados consecutivos | Orden correcto, título/reproductor coinciden, cada avance ocurre una vez. |
| Clientes | Dos overlays del mismo canal; cerrar el principal; skip sin overlay | Rol de reproductor definido y estado coherente; comando no depende de observador. |
| YouTube | Corto, no embebible, borrado, error de red, video >10 min | Detectar fin/error, política de duración clara y recuperación verificable. |
| Video directo | MP4/WebM HTTPS permitido, URL rota, codec no soportado | Reproducción o error controlado; no bloquear la cola. |
| Audio | Arranque sin click, 4 voces, 3 TTS seguidos, TTS durante música | Audio audible en OBS; TTS secuencial y sin promesas rechazadas ocultas. |
| Visibilidad | !von, !voff, !vstop y reconexión | Comportamiento definido y estado sincronizado tras reconectar. |
| Permisos | Viewer intenta comandos del streamer | Sin efectos administrativos. |
| Sorteo | Abrir, participar dos veces, cerrar, ganador vacío y válido | Participantes únicos y ganador dentro del conjunto. |
| Presentación | 1920×1080, 1280×720, 720×1280, nombres/textos largos | Sin cortes ni desbordamiento; legibilidad y transparencia correctas. |
| Texto externo | Tildes, emojis, etiquetas HTML y payload XSS de prueba | Se muestra texto literal, no se ejecuta HTML. |
| Estadísticas | Offline/en vivo, viewers cero, followers ausentes, API 401/429 | Mostrar estados fieles; no confundir suscriptores con seguidores. |
| Seguridad iframe | postMessage desde origen ajeno o ventana distinta | Ignorado; validar origen exacto y source del iframe. |

Realizar primero con mocks y después en un canal real controlado, incluyendo OBS. Una comprobación de sintaxis no demuestra reproducción ni autoplay.

### Fase 4: carga y recuperación

Objetivos iniciales propuestos, a ajustar según mediciones:

- Simular 10 canales × 2 sockets durante 30 minutos y un pico agregado de 20 mensajes/segundo durante 60 segundos usando eventos sintéticos. No bombardear Kick.
- Medir entrega de chat al overlay: p95 <1 segundo excluyendo el tiempo externo de Kick.
- Medir tiempo de generación TTS y espera en cola por separado; definir límites de texto, pendientes y concurrencia global tras medir CPU/RAM.
- Ejecutar una sesión controlada de 4 horas: sin crecimiento continuo de memoria/disco, sin workers duplicados, sin cruces entre canales.
- Cortar DB, Pusher, API Kick y proveedor TTS; verificar timeouts, backoff, estado visible y recuperación.
- Reiniciar/deplegar con cola activa; comprobar el comportamiento de persistencia acordado. Meta inicial de reconexión del overlay: <30 segundos desde que el servicio vuelve a estar listo.
- Restaurar un backup en una base aislada y comprobar recuperación de canales/configuración. Si la cola no se persiste, documentar la pérdida de forma explícita.

## Opciones de despliegue

### Opción recomendada: Render Docker y PostgreSQL persistente

El repositorio ya está preparado para Render. Usar un web service pago siempre activo, una sola instancia inicialmente y una base persistente con backups. El overlay y Socket.IO deben seguir en el mismo origen para simplificar el primer lanzamiento.

Elegir una de estas configuraciones, de forma explícita:

- Render + Supabase: DATABASE_URL como secreto; retirar del Blueprint la base `daibot-db` y su referencia `fromDatabase`. Verificar TLS y modo de pooler con la cadena real.
- Render + Render Postgres pago: conservar `fromDatabase` y elegir base persistente. Ubicar servicio y DB en la misma región cuando sea posible.

El README describe Supabase, pero `render.yaml` provisiona Render Postgres Free. Su comentario de 90 días está desactualizado: la documentación actual indica expiración a los 30 días. Los servicios web gratuitos pueden dormir tras 15 minutos sin tráfico entrante. Fuente: [Render Free](https://render.com/docs/free).

Render soporta Docker y WebSockets. Las conexiones se cierran cuando se reemplaza la instancia; preparar reconexión y sincronización. Fuente: [Docker](https://render.com/docs/docker), [WebSockets](https://render.com/docs/websocket).

### Alternativa: VPS con Docker Compose

Adecuado si quieres administrar infraestructura: contenedor Rust/TTS, proxy HTTPS como Caddy, PostgreSQL administrado o con volumen, firewall, backups externos y monitoreo. Requiere asumir actualizaciones, disponibilidad y restauración. Evitar depender del PC de streaming para el servicio compartido.

Comparar costo mensual total de CPU/RAM, base, backups y ancho de banda del TTS; no decidir solo por el precio del servidor. No se ha dimensionado infraestructura todavía.

### Escalado posterior

Antes de varias réplicas: persistir cola/versiones, compartir eventos entre procesos, definir dueño/lease por canal y serializar refresh OAuth. Sticky sessions por sí solas no resuelven los workers duplicados ni el estado en memoria. Evaluar Redis o una cola compartida solo cuando haya necesidad medida.

## Secuencia de lanzamiento y rollback

1. Corregir P0 y los P1 de autorización, lifecycle, errores DB y Unicode; decidir persistencia y transporte principal.
2. Añadir `/healthz` y `/readyz`, métricas de fallos/reconexiones, logs sin credenciales y apagado con cancelación de tareas. Configurar health check en Render: [documentación](https://render.com/docs/health-checks).
3. Ajustar Docker: build `--locked`, versión fijada de edge-tts, usuario sin privilegios y `.dockerignore`; verificar dependencias TLS en imagen final. Confirmar toolchain compatible con Cargo.lock.
4. Automatizar CI: compilación, tests, lint acordado y build Linux de la imagen. Desplegar un commit/image identificable a staging solo cuando pasan los checks.
5. Ejecutar fases 2 y 3 y probar redeploy/rollback. Los despliegues con solapamiento pueden duplicar lectores aun con una instancia nominal; usar un lease por canal o una ventana de mantenimiento controlada hasta resolverlo.
6. Lanzar primero con SeniorDai y después con 2–3 canales piloto. Supervisar un stream completo antes de ampliar.
7. Mantener la imagen anterior, backup y procedimiento de restauración probado. Migraciones compatibles con ambas versiones; rollback de imagen no revierte automáticamente la DB ni eventos externos ya enviados.

## Criterios para aprobar producción

- Compilación y suite pasan también en el entorno Linux del contenedor.
- Ningún P0 abierto; P1 funcionales resueltos o mitigados de forma verificable.
- Firmas inválidas, callbacks repetidos y escrituras sin autorización no producen efectos.
- Dos canales aislados, sin eventos duplicados y sin tareas antiguas tras reautorizar.
- OBS reproduce videos/TTS y recupera conexión; cola y reinicios tienen contrato explícito.
- DB persistente, backup restaurable, health checks y rollback probados.

Referencia de validación de firmas: [documentación oficial de Kick](https://github.com/KickEngineering/KickDevDocs/blob/main/events/webhook-security.md).

## Verificaciones de esta revisión

- `node --check overlay/app.js`: pasó.
- `cargo check --locked`: pasó en Windows con Rust 1.99.0. Esto comprueba el backend sin compilar la suite de tests ni ejecutar la aplicación.
- JavaScript interno de `pixel.html`: un script comprobado con `vm.Script` de Node, sintaxis válida.
- `cargo test --locked`: falló antes de ejecutar tests, con cinco errores de compilación en `queue/mod.rs` (dos usos de `fs` sin importar y tres llamadas a `VideoQueue::load` inexistente). No hay 33 tests aprobados; 33 es el recuento estático de funciones de prueba.
- Docker no está disponible en PATH en este entorno. No se ejecutó build de contenedor ni se publicó nada.
- No se ejecutó el backend conectado a producción; no se probaron credenciales reales, OAuth real, TTS externo ni OBS.
