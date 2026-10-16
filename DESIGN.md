---
name: DaiBotkick (CuchurruminRix)
description: Chasis brutalista gamer de placas blindadas biseladas, ángulos tácticos y bloques cromáticos planos para el overlay OBS 1920×1080 en vivo.
colors:
  electric-blue: "#1f22ff"
  deep-blue: "#1418b8"
  volt-yellow: "#ffe600"
  impact-magenta: "#ff2bd6"
  cyber-turquoise: "#2ff3e0"
  live-red: "#ff3b30"
  armor-ink: "#06061a"
  soft-ink: "#181818"
  paper-white: "#ffffff"
  lavender-mist: "#c9cbff"
  steel-muted: "#9b9b9b"
typography:
  display:
    fontFamily: "'Bangers', 'Impact', 'Haettenschweiler', sans-serif"
    fontSize: "36px"
    fontWeight: 400
    lineHeight: "40px"
    letterSpacing: "1.2px"
  headline:
    fontFamily: "'Bangers', 'Impact', 'Haettenschweiler', sans-serif"
    fontSize: "30px"
    fontWeight: 400
    lineHeight: "32px"
    letterSpacing: "1.5px"
  title:
    fontFamily: "'Bangers', 'Impact', 'Haettenschweiler', sans-serif"
    fontSize: "20px"
    fontWeight: 400
    lineHeight: "28px"
    letterSpacing: "1.5px"
  body:
    fontFamily: "'Manrope', 'Segoe UI', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    lineHeight: "28px"
  label:
    fontFamily: "'Manrope', 'Segoe UI', system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 700
    lineHeight: "16px"
    letterSpacing: "0.8px"
rounded:
  none: "0px"
  sm: "2px"
  md: "4px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  safe: "32px"
components:
  button-icon:
    backgroundColor: "{colors.deep-blue}"
    textColor: "{colors.paper-white}"
    rounded: "{rounded.sm}"
    padding: "6px"
    width: "38px"
    height: "38px"
  button-icon-hover:
    backgroundColor: "{colors.volt-yellow}"
    textColor: "{colors.armor-ink}"
  button-icon-active:
    backgroundColor: "{colors.impact-magenta}"
    textColor: "{colors.armor-ink}"
  button-anchor:
    backgroundColor: "{colors.armor-ink}"
    textColor: "{colors.paper-white}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "8px 14px"
    height: "48px"
  button-anchor-hover:
    backgroundColor: "{colors.volt-yellow}"
    textColor: "{colors.armor-ink}"
  button-anchor-active:
    backgroundColor: "{colors.impact-magenta}"
    textColor: "{colors.armor-ink}"
  button-disabled:
    backgroundColor: "{colors.soft-ink}"
    textColor: "{colors.steel-muted}"
  panel-armor:
    backgroundColor: "{colors.electric-blue}"
    textColor: "{colors.paper-white}"
    rounded: "{rounded.md}"
  chat-card-odd:
    backgroundColor: "{colors.paper-white}"
    textColor: "{colors.armor-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "18px 18px 12px"
  chat-card-even:
    backgroundColor: "{colors.volt-yellow}"
    textColor: "{colors.armor-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "18px 18px 12px"
  badge-status:
    backgroundColor: "{colors.armor-ink}"
    textColor: "{colors.cyber-turquoise}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "2px 10px"
---

# Design System: DaiBotkick (CuchurruminRix)

## Overview

**Creative North Star: "Chasis Industrial Biselado (Mecha Bay)"**

DaiBotkick proyecta la telemetría de un stream en vivo de Kick como un chasis blindado de bahía mecha: placas de acero esmaltadas en colores planos de alta energía, cortes diagonales a 45°, inclinaciones tácticas (`skewX`) y bordes continuos de tinta oscura (`3px` a `4px`). Cada módulo funciona como un instrumento físico acoplado al perímetro del lienzo 1920×1080, diseñado para que el espectador identifique en medio segundo quién habla en el chat, qué canción suena y qué bola acaba de salir en el bingo sin competir con la acción central del videojuego.

El sistema rechaza de forma explícita las tarjetas redondeadas genéricas de streaming, el glassmorphism translúcido, los degradados difusos en contenedores de interfaz y las sombras suaves con desenfoque. La profundidad y la jerarquía provienen exclusivamente del contraste cromático directo, los bloques desplazados en diagonal y las sombras sólidas sin difuminado (`0px` blur). El movimiento responde con masa física (`cubic-bezier(0.32, 0.72, 0, 1)`) y rebote mecánico contenido (`cubic-bezier(0.22, 1, 0.36, 1)`), desactivándose por completo bajo `prefers-reduced-motion: reduce`.

**Key Characteristics:**
- **Perímetro blindado de centro libre:** Toda la interfaz habita los márgenes seguros (`32px`) de un lienzo fijo 1920×1080 con fondo transparente para dejar despejada la partida.
- **Placas cromáticas planas sin degradados:** Superficies macizas en azul eléctrico, amarillo volt, magenta de impacto, turquesa cibernético, blanco papel y tinta negra.
- **Geometría en cizalla con compensación óptica:** Contenedores inclinados entre `-2deg` y `-10deg` (`skewX`) cuyos textos e iconos internos aplican contrainclinación positiva para preservar la lectura instantánea.
- **Bordes estructurales y sombras duras:** Contornos de tinta de `3px` a `4px` combinados con sombras desplazadas sólidas (`3px` a `14px`) que codifican categoría y estado.
- **Ilustración vectorial táctica y avatar brutalista:** Arte SVG local (`kinetic-burst.svg`) en esquinas y marcos brutalistas (`channel-avatar-sync`) que adaptan cualquier avatar del canal en la topbar, el cabezal del chat y la alerta con `pointer-events: none`.

## Colors

La paleta combina blindaje azul eléctrico y tinta profunda con tres acentos de señalización industrial (amarillo volt, magenta de impacto y turquesa cibernético) que operan como códigos de estado sobre vídeo en movimiento.

### Primary
- **Electric Armor Blue** (`electric-blue`): Superficie estructural primaria del chasis en la barra superior (`#hud`), paneles desplegables (`#queue-panel`, `#volume-panel`), reproductor multimedia (`#media-widget`), módulo de bingo (`#bingo-widget`) y caja de alertas (`#alert-box`).
- **Deep Cockpit Blue** (`deep-blue`): Subcapas empotradas dentro de módulos azules, botones de icono en reposo, pastillas del medidor de emojis, barra de metadatos de canción y celdas base del cartón ganador.

### Secondary
- **Volt Yellow** (`volt-yellow`): Acento de máxima prioridad visual para la placa de identidad del canal, títulos de ventana, tarjetas pares del chat, bolas cantadas del bingo, estados `:hover` tácticos y avisos de conexión.
- **Impact Magenta** (`impact-magenta`): Acento cinético de activación y énfasis para divisores diagonales, estados activos/expandidos (`[aria-expanded="true"]`, `[data-playing="true"]`), última bola cantada del bingo (`.latest`), etiquetas de consejos y sombras desplazadas de alto impacto.

### Tertiary
- **Cyber Turquoise** (`cyber-turquoise`): Señal de telemetría activa, foco de teclado (`:focus-visible`), barras de ecualizador, contadores de emojis/participantes, nombres de usuario en mensajes impares y estado de victoria verificada en el bingo (`data-phase="won"`).
- **Live Signal Red** (`live-red`): Reservado en la interfaz para el rombo pulsante de transmisión en directo (`.brand-live::before`) y detalles de impacto en activos ilustrados.

### Neutral
- **Armor Ink** (`armor-ink`): Tinta estructural para todos los bordes de `2px` a `4px`, cabeceras de paneles, contenedores de reloj y transporte, fondos de celdas pendientes del bingo, sombras base y texto oscuro sobre amarillo, blanco, magenta o turquesa.
- **Soft Disabled Ink** (`soft-ink`): Fondo apagado para interruptores desactivados (`[aria-pressed="false"]`).
- **Paper White** (`paper-white`): Superficie de máxima luminancia para mensajes impares del chat, tira de consejos inferiores (`.tip`), bola principal del sorteo (`#bingo-ball`) y texto primario sobre fondos azules o tinta.
- **Lavender Mist** (`lavender-mist`): Texto secundario legible sobre azul o tinta para fechas, subtítulos de ayuda y etiquetas de metadatos (`Pedido por`).
- **Muted Steel** (`steel-muted`): Color de texto e icono para controles con estado desactivado (`[aria-pressed="false"]`).

### Named Rules
**The Zero-Gradient Surface Rule.** Ningún contenedor, botón, tarjeta o insignia de la interfaz utiliza degradados CSS ni transparencias difusas; cada plano es un bloque sólido delimitado por un borde de tinta.

**The Chromatic State-Shadow Rule.** El color de la sombra sólida desplazada anuncia el rol o estado del bloque: tinta (`armor-ink`) para estructura principal, azul (`electric-blue`) para tarjetas blancas en reposo, magenta (`impact-magenta`) para paneles activos o tarjetas amarillas, amarillo (`volt-yellow`) para el reproductor musical y turquesa (`cyber-turquoise`) cuando el bingo entra en fase `won`.

**The Ink-On-Bright Contrast Rule.** Todo texto o icono sobre superficies `volt-yellow`, `impact-magenta`, `cyber-turquoise` o `paper-white` se compone obligatoriamente en `armor-ink` (con realces puntuales en `electric-blue`), nunca en blanco.

## Typography

**Display Font:** `Bangers` (con fallback a `'Impact', 'Haettenschweiler', sans-serif`)
**Body Font:** `Manrope` (con fallback a `'Segoe UI', system-ui, sans-serif`)
**Emoji Font:** `'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji'`

**Character:** `Bangers` aporta el golpe gráfico de cómic arcade en mayúsculas para cabeceras de blindaje, identidad de canal y alertas, mientras `Manrope` en pesos semibold y bold (`600`–`700`) con números tabulares (`tabular-nums`) garantiza lectura limpia a distancia en el chat, el reloj y la matriz numérica del bingo.

### Hierarchy
- **Display** (`400`, `36px`, `40px` line-height, `1.2px` letter-spacing, uppercase): Cuerpo principal de alertas en vivo (`.alert-body`) con sombra dura de texto (`4px 4px 0`).
- **Headline** (`400`, `30px`, `32px` line-height, `1.5px` letter-spacing, uppercase): Nombre del canal en la placa de marca superior (`.stream-title`).
- **Title** (`400`, `20px`, `28px` line-height, `1.2px`–`1.5px` letter-spacing, uppercase): Cabeceras de módulos (`.aero-window-title`, `.widget-heading`). En pastillas de consejo (`.tip-label`) desciende a `16px` / `24px` con `1px` de tracking.
- **Body** (`700`, `20px`, `28px` line-height): Mensajes del chat en vivo (`.msg-text`, truncados a 2 líneas) y títulos de vídeo (`#video-title`). Variante de `18px` / `26px`–`28px` (`700`) para estados vacíos del chat, estado del bingo, barra de consejos y avisos de conexión; `16px` / `24px` (`600`–`700`) para listas de cola, solicitante de música y bola activa del bingo.
- **Label** (`700`, `12px`–`14px`, `16px`–`20px` line-height, `0.5px`–`1.2px` letter-spacing, uppercase): Reloj secundario/fecha, medidor de emojis, anclas de panel (`bingo`, `music`), nombres de usuario en el chat (`.msg-user`), contadores de bingo y celdas del tablero 5×15.

### Named Rules
**The Tabular Telemetry Rule.** El lienzo declara `font-variant-numeric: tabular-nums` en `body` para que el reloj en vivo, los porcentajes de volumen, el contador de emojis y las 75 celdas del bingo nunca desplacen el layout al actualizarse.

**The Counter-Skew Legibility Rule.** Cuando una placa contenedora aplica una cizalla pronunciada (`skewX(-6deg)` a `skewX(-10deg)`), el texto de lectura continua o el icono interior compensa el ángulo con una transformación opuesta (`skewX(4deg)` a `skewX(14deg)`) para evitar fatiga visual en stream.

## Layout

El overlay opera sobre un lienzo fijo de `1920px × 1080px` (`overflow: hidden`, fondo `transparent`) escalado proporcionalmente en JavaScript mediante `transform: scale(min(innerWidth / 1920, innerHeight / 1080))` con `transform-origin: top left`. No existen puntos de ruptura responsivos por reflujo de columnas: la geometría preserva las proporciones exactas de OBS en cualquier ventana.

- **Perímetro seguro (`--safe: 32px`):** Todos los paneles principales se anclan a `32px` de los bordes izquierdo y derecho, dejando una zona central libre de más de `960px` de ancho para el videojuego.
- **Barra de sistema superior (`#hud`):** Anclada en `top: 16px`, `left: 32px`, `right: 32px`, con `height: 76px`, `padding: 8px 16px 8px 12px` y `gap: 14px`.
- **Columna táctica izquierda:** El módulo de bingo (`#bingo-widget`) ocupa `left: 32px`, `top: 112px`, `width: 440px`; el widget de chat (`#chat-widget`) se sitúa en `left: 32px`, `bottom: 88px`, `width: 488px`, `max-height: 492px` con pila vertical de hasta 4 mensajes (`gap: 18px`).
- **Columna táctica derecha:** El reproductor multimedia (`#media-widget`) se ancla en `right: 32px`, `top: 112px`, `width: 408px` con contenedor de vídeo `16:9` y margen interno de `16px`. Los paneles flotantes de cola y volumen (`.topbar-panel`) abren en `top: 108px`, `right: 160px`, `width: 380px`, `max-height: 420px`.
- **Franja inferior y esquinas cinéticas:** La tira de consejos (`#tips`) se posiciona en `right: 56px`, `bottom: 26px` (`width: 900px`, `height: 58px`). Las ráfagas decorativas (`.kinetic-corner`) ocupan `360px × 68px` en `bottom: 0` a izquierda y derecha sin invadir el área interactiva.

## Elevation & Depth

El sistema es estrictamente plano en sus superficies y construye profundidad mediante **capas cromáticas recortadas en tinta** y **sombras duras desplazadas sin desenfoque (`0px` blur)**. Nunca se emplean sombras ambientales difusas ni filtros de desenfoque en el DOM de la interfaz.

### Shadow Vocabulary
- **Alert Impact Offset** (`box-shadow: 14px 14px 0 #ff2bd6`): Elevación máxima del sistema, reservada para la placa central de alertas (`#alert-box`) al irrumpir sobre la partida.
- **Primary Chassis Offset** (`box-shadow: 10px 10px 0 <color>`): Elevación estructural de los grandes módulos del perímetro: en `#hud` usa `#06061a`; en `#bingo-widget` y `.topbar-panel` usa `#ff2bd6` (cambiando a `#2ff3e0` cuando el bingo es ganado); en `#media-widget` usa `#ffe600`.
- **Card & Banner Offset** (`box-shadow: 8px 8px 0 <color>`): Elevación estándar (`--hard`) para mensajes del chat (`#1f22ff` en impares, `#ff2bd6` en pares), tira de consejos (`#1f22ff`) y banner de estado de conexión (`#06061a`).
- **Header Plate Offset** (`box-shadow: 6px 6px 0 <color>`): Placa de canal `.topbar-start` (`#ff2bd6`) y cabecera `.widget-heading` del chat (`#1f22ff`).
- **Control & Drum Offset** (`box-shadow: 4px 4px 0 <color>`): Botones ancla en reposo (`#2ff3e0`), botones de icono en `:hover` (`#ff2bd6`), estados activos (`#ffe600`) y bola central de bingo (`#06061a`).
- **Badge Micro-Offset** (`box-shadow: 3px 3px 0 <color>`): Placas de nombre de usuario `.msg-user`, indicador `+N en cola` y etiquetas `.tip-label`.

### Named Rules
**The Hard-Cut Press Rule.** Al elevarse en `:hover`, los controles tácticos se desplazan `-3px, -3px` y proyectan una sombra sólida de `4px` a `5px` en magenta; en `:active`, se hunden `translateY(1px)` y eliminan por completo su `box-shadow` (`none`), simulando un interruptor mecánico físico.

## Shapes

El lenguaje formal sustituye las curvas suaves por **cortes biselados, paralelogramos en cizalla y remates poligonales**:

- **Radios mínimos de estampado (`2px` a `4px`):** Los contenedores mayores (`#hud`, `#media-widget`, `#bingo-widget`, `.topbar-panel`, `#alert-box`) usan `border-radius: 4px`; botones, tarjetas de chat, insignias y celdas usan `border-radius: 2px`; divisores, adornos de cabecera y barras de ecualizador usan `0px`.
- **Contornos de tinta (`2px` a `4px`):** Módulos principales, tarjetas de chat y placas de cabecera llevan `border: 4px solid #06061a`; botones tácticos, avatares, marcos de vídeo y placas de usuario llevan `3px solid #06061a`; insignias compactas y celdas cabeceras del bingo llevan `2px solid #06061a`.
- **Inclinación táctica (`skewX` y `rotate`):** La barra superior se inclina `skewX(-6deg)`, los mensajes del chat alternan `skewX(-4deg)` y `skewX(-6deg) translateX(6px)`, y las placas destacadas combinan cizalla con rotación leve (`skewX(-6deg) rotate(-2deg)` en `.topbar-start`, `skewX(-10deg) rotate(-2deg)` en `.widget-heading`).
- **Recortes poligonales y chaflanes a 45°:** Los paneles laterales (`#media-widget::before`, `#bingo-widget::before`) incorporan una placa cuadrada de `36px × 36px` rotada `45deg` en la esquina inferior derecha para crear un chaflán blindado; los mensajes del chat despliegan una cola triangular lateral mediante `clip-path: polygon(100% 0, 0 35%, 100% 100%)`; el bombo del bingo superpone octógonos recortados con `clip-path: polygon(30% 0, 70% 0, 100% 30%, 100% 70%, 70% 100%, 30% 100%, 0 70%, 0 30%)`.

## Components

### Buttons
Botones mecánicos de control rápido pensados para el modo **Interactuar** de OBS.
- **Shape:** Esquinas microbiseladas (`2px` radius) con borde de tinta de `3px` (`#06061a`).
- **Primary (Icon Tactical Button `.topbar-button.aero-icon-button`):** Cuadrado de `38px × 38px`, `padding: 6px`, fondo `deep-blue` (`#1418b8`), icono SVG de `20px × 20px` en `paper-white` con contrainclinación `skewX(12deg)`.
- **Panel Anchor (`.music-anchor`):** Botón rotulado de `height: 48px`, `padding: 8px 14px`, `gap: 8px`, fondo `armor-ink` (`#06061a`), texto `14px` uppercase bold en `paper-white`, inclinación `skewX(-8deg)` y sombra `4px 4px 0 #2ff3e0`.
- **Hover / Focus / Active:** En `:hover`, fondo `volt-yellow` (`#ffe600`), texto/icono `armor-ink` (`#06061a`), `transform: translate(-3px, -3px)` y sombra `4px 4px 0` (o `5px 5px 0`) en `#ff2bd6`. En `:active`, `transform: translateY(1px)` sin sombra. En `:focus-visible`, `outline: 3px solid #2ff3e0` con `outline-offset: 4px`.
- **Expanded / Playing / Disabled States:** Cuando `[aria-expanded="true"]` o `[data-playing="true"]`, el fondo pasa a `impact-magenta` (`#ff2bd6`) con texto `#06061a` y sombra `4px 4px 0 #ffe600`. Cuando `[aria-pressed="false"]`, el fondo cae a `soft-ink` (`#181818`), texto `steel-muted` (`#9b9b9b`) y sombra `2px 2px 0 #06061a`. El atributo `:disabled` aplica `opacity: 0.4`.

### Chips
Placas angulares compactas para atribución de usuario, telemetría y conteo en vivo.
- **Style:** Borde de `2px` o `3px` en `#06061a`, `border-radius: 2px`, inclinación `skewX(-6deg)` a `skewX(-12deg)`.
- **State & Variants:**
  - **User Callout (`.msg-user`):** Flota a `top: -16px`, `left: 14px` sobre la tarjeta del mensaje; fondo `#06061a`, texto `#2ff3e0` con sombra `3px 3px 0 #ff2bd6` en mensajes impares, y texto `#ffe600` con sombra `3px 3px 0 #1f22ff` en mensajes pares. Incluye un marcador triangular (`clip-path: polygon(0 0, 100% 50%, 0 100%)`).
  - **Live Status (`.brand-live`):** Fondo `#06061a`, texto `#2ff3e0` (`12px` bold uppercase) y rombo rojo pulsante (`8px × 8px`, `#ff3b30`, `rotate(45deg)`).
  - **Telemetry & Queue Badges (`#bingo-participants`, `#queue-indicator`, `.bingo-caption`):** Combinan `#06061a`/`#2ff3e0` para participantes, `#ff2bd6`/`#06061a` con sombra `#ffe600` para cola pendiente, y `#ffe600`/`#06061a` para el progreso `0 / 75`.

### Cards / Containers
Módulos blindados perimetrales y tarjetas escalonadas de conversación.
- **Corner Style:** `4px` en paneles de módulo; `2px` en tarjetas de chat y tiras de consejo.
- **Background:** `electric-blue` (`#1f22ff`) para paneles de sistema; alternancia de `paper-white` (`#ffffff`) y `volt-yellow` (`#ffe600`) en tarjetas `.msg` del chat.
- **Shadow Strategy:** Sombra dura de `10px 10px 0` en módulos laterales y desplegables; `8px 8px 0` en tarjetas de chat y consejos.
- **Border:** `4px solid #06061a` en todo el perímetro, con barra de título superior `.aero-window-title` en `#06061a`, texto `#ffe600`, filete inferior amarillo de `120px × 4px` y destello magenta inclinado (`.aero-window-glint`, `44px × 10px`, `skewX(-32deg)`). Las esquinas de los módulos principales (`#media-widget`, `#bingo-widget`, `#alert-box`, `.widget-heading`, `.tip` y `.kinetic-corner`) incorporan arcos eléctricos ramificados en SVG (`--arc-corner-cyan` y `--arc-corner-volt`) que abrazan los bordes en L con núcleo blanco incandescente, halo turquesa/volt, filamentos secundarios y contorno de tinta `#06061a`.
- **Internal Padding:** `14px 16px 16px` en el cuerpo de paneles laterales; `16px 24px` en paneles desplegables; `18px 18px 12px` en tarjetas de chat.

### Inputs / Fields
Control deslizante de volumen local de música dentro de `#volume-panel`.
- **Style:** `input[type="range"]` de ancho completo (`width: 100%`, `margin: 16px 0`) con `accent-color: #ffe600` sobre el fondo azul del panel y etiqueta superior en mayúsculas con valor numérico tabular (`output#volume-value`).
- **Focus:** `outline: 3px solid #2ff3e0` con `outline-offset: 4px` en `:focus-visible`.

### Navigation
Barra táctica superior `#hud.system-topbar` que concentra identidad, reloj, telemetría de emojis, transporte musical y conmutadores de visibilidad.
- **Style & Typography:** Contenedor flex horizontal (`height: 76px`, `skewX(-6deg)`) en `#1f22ff` con borde `4px solid #06061a`, sombra `10px 10px 0 #06061a` y dos arcos eléctricos brutalistas en las esquinas (`::before` amarillo arriba a la derecha y `::after` turquesa abajo a la izquierda, animados con `volt-arc`).
- **Clusters:** Agrupa la placa de canal `.topbar-start` (en `#ffe600` con avatar de `46px × 46px`), divisor magenta inclinado (`8px × 42px`, `skewX(-22deg)`), reloj tabular `.topbar-clock`, medidor `#emoji-meter` y clústeres de botones `.aero-control-cluster` sobre fondo `#06061a`.

### Bingo 75-Ball Telemetry Matrix & Drum
Componente insignia del overlay (`#bingo-widget`) que visualiza sorteos verificados en servidor sin tapar el chat.
- **Tactical Drum (`.bingo-drum`):** Roseta de `84px × 84px` formada por tres octógonos concéntricos (`#06061a`, `#ffe600`, `#ff2bd6`) que giran en sentidos alternos (`burst-turn`), coronados por la bola actual `#bingo-ball` (`58px × 58px`, fondo `#ffffff`, borde `4px solid #06061a`, rotación `-6deg` y animación de entrada `bingo-arrival`).
- **5×15 Board (`.bingo-board-row`):** Cinco filas encabezadas por las letras `B-I-N-G-O` en celdas amarillas de `22px × 24px` seguidas de 15 celdas numéricas (`height: 24px`, `gap: 2px`). Las bolas pendientes usan fondo `#06061a` y texto `#c9cbff`; las sorteadas (`.called`) cambian a fondo `#ffe600` y texto `#06061a`; la última bola (`.latest`) destaca en `#ff2bd6` con `scale(1.18) rotate(-4deg)` y anillo `0 0 0 2px #06061a`.
- **Verified Winner Card (`#bingo-winning-card`):** Cuando `data-phase="won"`, el tablero de 75 bolas y el historial se ocultan automáticamente y son reemplazados por una cuadrícula `5×5` (`border: 4px solid #06061a`, fondo `#06061a`) donde las celdas de la línea ganadora brillan en `#2ff3e0` con texto `#06061a`.

## Do's and Don'ts

### Do:
- **Do** mantener todos los módulos, alertas y adornos dentro del perímetro del lienzo `1920×1080` respetando el margen `--safe: 32px` y el centro transparente para el juego.
- **Do** enmarcar cada contenedor principal con `border: 4px solid #06061a`, `border-radius: 2px`–`4px` y sombra dura desplazada (`0px` blur).
- **Do** compensar la inclinación (`skewX`) de contenedores densos aplicando una contrainclinación positiva en los textos e iconos hijos.
- **Do** insertar cualquier texto procedente de usuarios, chat, canciones o servidor mediante `textContent` o nodos de texto seguros, truncando mensajes largos y aplicando `overflow-wrap: anywhere`.
- **Do** usar las curvas de movimiento con masa física (`--mass: cubic-bezier(0.32, 0.72, 0, 1)` y `--spring: cubic-bezier(0.22, 1, 0.36, 1)`) y desactivar toda animación bajo `@media (prefers-reduced-motion: reduce)`.

### Don't:
- **Don't** usar `backdrop-filter: blur()`, fondos translúcidos, sombras difusas ni degradados en paneles, tarjetas o botones de la interfaz.
- **Don't** redondear contenedores o controles por encima de `4px` ni usar pastillas (`9999px`) o avatares circulares (`50%`).
- **Don't** colocar texto blanco (`#ffffff`) directamente sobre superficies `volt-yellow` (`#ffe600`), `cyber-turquoise` (`#2ff3e0`) o `impact-magenta` (`#ff2bd6`); usa siempre `armor-ink` (`#06061a`).
- **Don't** permitir que ilustraciones decorativas (`.kinetic-corner`, `.heading-sticker`, `.alert-art`) capturen eventos de puntero; todas deben declarar `pointer-events: none`.
