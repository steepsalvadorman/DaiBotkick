# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users
Streamers hispanohablantes en Kick y sus comunidades en vivo. El streamer opera el canal durante partidas o transmisiones en directo usando OBS Studio (Browser Source 1920×1080) y necesita que el overlay proyecte el pulso del chat, la cola musical, alertas, TTS y partidas de bingo sin tapar la acción central del juego ni requerir administración constante. Los espectadores interactúan desde el chat de Kick mediante comandos (`!play`, `!dai`, `!carton`, `!bingo`, `!participar`) y consultan su cartón personal de bingo en el navegador.

## Product Purpose
DaiBotkick (**CuchurruminRix**) es un bot multicanal para Kick con cola de música/video de YouTube, voces TTS oficiales y personalizadas, sorteos, bingo interactivo de 75 bolas verificadas en servidor y un overlay OBS en tiempo real. Su propósito es convertir la actividad del chat en una experiencia audiovisual compartida dentro del stream con mínima fricción operativa.

## Positioning
Un sistema integrado en Rust (Axum + Socket.IO + PostgreSQL) conectado exclusivamente por webhooks EventSub firmados por RSA de Kick, donde un único socket de overlay autorizado mediante `#token` controla la reproducción de video/TTS y el avance validado de la cola (por ID y versión), mientras las vistas públicas sin token pueden observar el chat, estadísticas y el bingo sin interferir con la reproducción del canal.

## Operating Context
- Lienzo fijo de 1920×1080 en OBS Browser Source con fondo transparente para dejar libre el centro del videojuego (sin marco de cámara).
- Interacción ocasional del streamer mediante el modo **Interactuar** de OBS sobre la barra superior (`#hud`) para pausar/reanudar/saltar canciones, abrir la cola, ajustar el volumen local de música (sin afectar TTS) o alternar visibilidad de paneles (`chat`, `music`, `bingo`).
- Entorno en vivo de alto contraste donde los mensajes del chat, emotes animados de Kick (`[emote:ID:NOMBRE]`), alertas de comunidad y bolas de bingo deben leerse al instante sobre fondos de juego cambiantes.

## Capabilities and Constraints
- **Seguridad XSS estricta:** Todo contenido externo (nombres de usuario, mensajes de chat, títulos de canciones, alertas, ganadores) se inserta exclusivamente como texto literal (`textContent` / nodos de texto), nunca mediante `innerHTML`.
- **Control de reproducción único:** Solo el overlay con `#token` válido y rol activo (`playbackRole.active = true`) reproduce YouTube, video directo y TTS, y emite `advanceQueue` con `{ id, version }`.
- **Límites y resiliencia:** Límite de 10 minutos por video; reconexión automática de Socket.IO; degradación explícita si falla un emote de Kick (reemplazo por `:NOMBRE:` y aviso en estado); medidor de emojis y emotes en ventana móvil de 5 minutos (actualizado cada 5 s).
- **Bingo de 75 bolas:** Tablero 5×15 en el overlay con sustitución automática del tablero e historial por el cartón ganador verificado (`5×5`) cuando la fase pasa a `won`, evitando tapar el chat en la esquina inferior izquierda.
- **Accesibilidad de movimiento:** Soporte completo para `prefers-reduced-motion: reduce`, desactivando animaciones y mostrando únicamente el primer consejo en `#tips`.

## Brand Commitments
- **Mascota e identidad:** **CuchurruminRix**, un ninja veloz electrónico y programador con identidad gamer (`overlay/cuchurruminrix.svg`), acompañado de ilustraciones vectoriales locales originales (`overlay/volt-hand.svg`, `overlay/kinetic-burst.svg`) que no dependen de URLs externas ni interceptan clics.
- **Dirección estética confirmada por el usuario:** Estilo **Tech-Brutalist Gamer HUD** con mejores ángulos (cortes biselados/diagonales tácticas, inclinaciones agresivas, bloques macizos, bordes de tinta completos y sombras duras desplazadas) manteniendo la paleta plana de alto contraste Volt/Cyber (`#1f22ff` azul eléctrico, `#1418b8` azul profundo, `#ffe600` amarillo volt, `#ff2bd6` magenta, `#2ff3e0` turquesa, `#ff3b30` rojo vivo, `#06061a` tinta negra y `#ffffff` papel).
- **Tipografía:** Combinación de `Bangers` (display/rótulos/marca/alertas) y `Manrope` (cuerpo/chat/controles/datos tabulares) cargadas desde Google Fonts con `display=swap`.

## Evidence on Hand
- Overlay principal en `overlay/pixel.html`, `overlay/style.css` y `overlay/app.js` (con redirección en `overlay/index.html`).
- Activos vectoriales originales en `overlay/cuchurruminrix.svg`, `overlay/volt-hand.svg` y `overlay/kinetic-burst.svg`.
- Cartón personal de bingo en `overlay/bingo.html`, `overlay/bingo.css` y `overlay/bingo.js`.
- Suite de pruebas automatizadas del overlay en `tests/overlay.test.cjs` que valida el comportamiento de transporte, seguridad XSS, emotes, bingo, invariantes de tipografía/escala/curvas de movimiento y estructura DOM.

## Product Principles
1. **El juego manda en el centro:** Todo panel, alerta, esquina decorativa y notificación se mantiene en el perímetro del lienzo 1920×1080 para no invadir la partida.
2. **Lectura instantánea bajo fuego:** Contraste brutalista extremo, bloques planos sin degradados difusos y jerarquía tipográfica clara para que un vistazo de medio segundo baste en pleno stream.
3. **Autoridad de reproducción inquebrantable:** Separación estricta entre el reproductor privado con token y las vistas públicas de solo lectura.
4. **Seguridad y honestidad visual:** Cero HTML no confiable y cero adornos que bloqueen clics o simulen métricas falsas.
