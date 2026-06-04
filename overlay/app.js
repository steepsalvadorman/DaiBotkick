// ─── SeniorDai — Overlay v4.2 (Direct iframe, no YT.Player API) ──────────────
// Protocolo:
//   Servidor → "syncQueue"    (array completo de la cola)
//   Servidor → "nextVideo"    (skip manual del owner)
//   Overlay  → "advanceQueue" (video terminado)

// Canal: leer de ?ch=slug en la URL
const _ch = new URLSearchParams(location.search).get('ch') || '';
const socket = _ch ? io({ query: { ch: _ch } }) : io();

// ── AudioContext (debe ir ANTES de cualquier uso) ─────────────────────────────
const audioContext   = new (window.AudioContext || window.webkitAudioContext)();
const systemAnalyser = audioContext.createAnalyser();
systemAnalyser.fftSize = 64;
systemAnalyser.smoothingTimeConstant = 0.8;

// ── DOM Refs ──────────────────────────────────────────────────────────────────
const chatMessages     = document.getElementById("chat-messages");
const mediaWidget      = document.getElementById("media-widget");
const videoTitleEl     = document.getElementById("video-title");
const videoRequesterEl = document.getElementById("video-requester");
const youtubePlayerDiv = document.getElementById("youtube-player");
const directVideo      = document.getElementById("direct-video");

// ── Configuración de volumen ──────────────────────────────────────────────────
const VOL_MUSIC_DIR = 0.25;
const VOL_TTS       = 1.0;

// ── Estado ────────────────────────────────────────────────────────────────────
let audioUnlocked = true;
audioContext.resume();

function unlockAudio() {} // no-op, mantenido por compatibilidad

let localQueue     = [];
let isPlaying      = false;
let isVideoVisible = true;
let isAdvancing    = false;
let loadingTimeout;
let ytMsgHandler   = null;

// ── syncQueue — evento principal ──────────────────────────────────────────────
socket.on("syncQueue", (queue) => {
    console.log(`📋 syncQueue recibido: ${queue.length} videos`);
    localQueue = queue;
    updateQueueBadge();

    if (localQueue.length === 0) {
        isPlaying   = false;
        isAdvancing = false;
        hideWidget();
        youtubePlayerDiv.innerHTML = "";
        if (directVideo) { directVideo.pause(); directVideo.src = ""; }
    } else if (!isPlaying) {
        playHead();
    } else if (youtubePlayerDiv.children.length === 0 &&
               !(directVideo.src && !directVideo.paused)) {
        // isPlaying=true pero nada reproduciéndose realmente → recuperar
        isPlaying = false;
        playHead();
    }
});

socket.on("nextVideo", () => {
    console.log("⏭ Skip manual recibido");
    finishCurrent();
});

socket.on("toggleVideo", (data) => {
    console.log(`👁️ toggleVideo: ${data.showVideo}`);
    isVideoVisible = data.showVideo;
    if (!isVideoVisible) {
        if (mediaWidget) mediaWidget.style.display = "none";
    } else if (isPlaying && mediaWidget) {
        mediaWidget.style.display = "block";
        requestAnimationFrame(() => {
            mediaWidget.style.opacity   = "1";
            mediaWidget.style.transform = "scale(1)";
        });
    }
});

// ── Lógica de reproducción ────────────────────────────────────────────────────
function playHead() {
    if (isPlaying || localQueue.length === 0 || !audioUnlocked) return;
    const next = localQueue[0];
    if (!next.videoId && !next.url) {
        console.warn("⚠️ Item sin videoId ni url — saltando directamente");
        socket.emit("advanceQueue", { videoId: null, url: null });
        return;
    }
    isPlaying = true;
    console.log(`🎬 Reproduciendo: "${next.title}" — por ${next.user}`);
    showWidget(next);
    if (next.videoId) playYouTube(next.videoId);
    else              playDirect(next.url);
}

function finishCurrent() {
    if (isAdvancing || localQueue.length === 0) return;
    isAdvancing = true;
    setTimeout(() => isAdvancing = false, 1000);
    clearTimeout(loadingTimeout);
    if (ytMsgHandler) { window.removeEventListener("message", ytMsgHandler); ytMsgHandler = null; }
    isPlaying = false;
    const item = localQueue[0];
    socket.emit("advanceQueue", { videoId: item?.videoId ?? null, url: item?.url ?? null });
}

// ── YouTube (iframe directo, sin YT.Player API) ───────────────────────────────
// Arranca muted=1 para garantizar autoplay en OBS CEF,
// luego manda unMute vía postMessage cuando el player esté listo.
function playYouTube(videoId) {
    directVideo.pause();
    directVideo.style.display  = "none";
    youtubePlayerDiv.innerHTML = "";
    youtubePlayerDiv.style.display = "block";

    const iframe = document.createElement("iframe");
    iframe.style.cssText = "width:100%;height:100%;border:none;display:block;";
    iframe.allow = "autoplay; encrypted-media";
    // mute=1 asegura autoplay en navegadores/OBS con restricción de autoplay
    iframe.src = `https://www.youtube.com/embed/${encodeURIComponent(videoId)}` +
        `?autoplay=1&mute=1&controls=0&rel=0&modestbranding=1&enablejsapi=1` +
        `&origin=${location.origin}`;
    youtubePlayerDiv.appendChild(iframe);

    // Desmutear y solicitar play explícito después de que el iframe cargue
    iframe.addEventListener("load", () => {
        setTimeout(() => {
            try {
                iframe.contentWindow.postMessage(
                    JSON.stringify({ event: "command", func: "unMute",    args: "" }), "*");
                iframe.contentWindow.postMessage(
                    JSON.stringify({ event: "command", func: "playVideo", args: "" }), "*");
                iframe.contentWindow.postMessage(
                    JSON.stringify({ event: "command", func: "setVolume", args: [100] }), "*");
            } catch {}
        }, 800);
    });

    // Detectar fin y errores via postMessage
    if (ytMsgHandler) window.removeEventListener("message", ytMsgHandler);
    ytMsgHandler = (e) => {
        if (!String(e.origin).includes("youtube")) return;
        try {
            const d = JSON.parse(e.data);
            if (d.event === "onStateChange" && d.info === 0) finishCurrent(); // ended
            if (d.event === "onError") finishCurrent();
        } catch {}
    };
    window.addEventListener("message", ytMsgHandler);

    clearTimeout(loadingTimeout);
    loadingTimeout = setTimeout(finishCurrent, 600000); // 10min max
}

function playDirect(url) {
    youtubePlayerDiv.style.display = "none";
    youtubePlayerDiv.innerHTML     = "";
    directVideo.style.display      = "block";
    directVideo.src    = url;
    directVideo.volume = VOL_MUSIC_DIR;
    directVideo.play().catch(() => finishCurrent());
    directVideo.onended = () => finishCurrent();
    directVideo.onerror = () => finishCurrent();
}

// ── Widget UI ─────────────────────────────────────────────────────────────────
function showWidget(data) {
    if (isVideoVisible && mediaWidget) {
        mediaWidget.style.display = "block";
        void mediaWidget.offsetWidth;
    }
    if (videoTitleEl) {
        const titleText = data.title || "Sin título";
        videoTitleEl.classList.remove("scrolling");
        videoTitleEl.innerHTML = `<span class="title-inner" style="display:inline-block;white-space:nowrap;">${titleText}</span>`;
        setTimeout(() => {
            const innerSpan = videoTitleEl.querySelector(".title-inner");
            if (innerSpan) {
                const isOverflowing = innerSpan.scrollWidth > videoTitleEl.clientWidth || titleText.length > 32;
                if (isOverflowing) {
                    videoTitleEl.classList.add("scrolling");
                    innerSpan.innerText = `${titleText}   ★★★   ${titleText}`;
                    innerSpan.classList.add("marquee");
                }
            }
        }, 150);
    }
    if (videoRequesterEl) videoRequesterEl.innerText = data.user || "Anónimo";
    if (isVideoVisible && mediaWidget) {
        requestAnimationFrame(() => {
            mediaWidget.style.opacity   = "1";
            mediaWidget.style.transform = "scale(1)";
        });
    }
}

function hideWidget() {
    mediaWidget.style.opacity   = "0";
    mediaWidget.style.transform = "scale(0.9)";
    setTimeout(() => { if (localQueue.length === 0) mediaWidget.style.display = "none"; }, 500);
}

function updateQueueBadge() {
    const badge = document.getElementById("queue-indicator");
    if (!badge) return;
    if (localQueue.length > 1) {
        badge.style.display = "inline-block";
        badge.innerText     = `+${localQueue.length - 1} en cola`;
    } else {
        badge.style.display = "none";
    }
}

// ── Weather ───────────────────────────────────────────────────────────────────
async function updateWeather() {
    try {
        const res  = await fetch("https://wttr.in/Lima?format=j1");
        const data = await res.json();
        const cur  = data.current_condition[0];
        const t = document.getElementById("weather-temp");
        const c = document.getElementById("weather-cond");
        if (t) t.innerText = `${cur.temp_C}°C`;
        if (c) c.innerText = cur.lang_es ? cur.lang_es[0].value : cur.weatherDesc[0].value;
    } catch {}
}
updateWeather();
setInterval(updateWeather, 900000);

// ── Audio Visualizer ──────────────────────────────────────────────────────────
(async () => {
    try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        audioContext.createMediaStreamSource(stream).connect(systemAnalyser);
    } catch { console.warn("Stereo Mix no disponible"); }
})();

const musicCanvas = document.getElementById("module-visualizer");
if (musicCanvas) {
    const ctx = musicCanvas.getContext("2d");
    const buf = new Uint8Array(systemAnalyser.frequencyBinCount);
    (function draw() {
        requestAnimationFrame(draw);
        systemAnalyser.getByteFrequencyData(buf);
        musicCanvas.width  = musicCanvas.offsetWidth;
        musicCanvas.height = musicCanvas.offsetHeight;
        const w = musicCanvas.width, h = musicCanvas.height;
        ctx.clearRect(0, 0, w, h);
        const bw = (w / buf.length) * 1.5;
        let x = 0;
        for (let i = 0; i < buf.length; i++) {
            const bh = (buf[i] / 255) * h;
            ctx.fillStyle = `hsla(${260 + i * 2}, 80%, 70%, 0.4)`;
            ctx.fillRect(x, h - bh, bw - 1, bh);
            x += bw;
        }
    })();
}

// ── Socket Events ─────────────────────────────────────────────────────────────
socket.on("sysStats", (data) => {
    const c = document.getElementById("cpu-load");
    const r = document.getElementById("ram-usage");
    if (c) c.innerText = `${data.cpu}%`;
    if (r) r.innerText = `${Math.round(data.ram)}%`;
});

socket.on("chatMessage", (data) => {
    if (!chatMessages) return;
    const div = document.createElement("div");
    div.style.cssText = "margin-bottom:12px;display:flex;flex-direction:column;animation:slideIn 0.3s ease-out forwards;";
    div.innerHTML = `
        <div style="font-weight:800;font-size:0.85rem;color:#53fc18;text-transform:uppercase;letter-spacing:0.5px;margin-bottom:3px;">${data.user}</div>
        <div style="background:rgba(255,255,255,0.05);padding:8px 12px;border-radius:0 12px 12px 12px;font-size:1rem;line-height:1.4;border-left:3px solid #53fc18;">${data.content}</div>
    `;
    chatMessages.appendChild(div);
    if (chatMessages.children.length > 8) chatMessages.removeChild(chatMessages.firstChild);
    chatMessages.scrollTop = chatMessages.scrollHeight;
});

socket.on("speak", (data) => {
    if (audioContext.state === "suspended") audioContext.resume();
    const ttsAudio = new Audio("data:audio/mp3;base64," + data.audioBase64);
    ttsAudio.volume = VOL_TTS;
    ttsAudio.play();
});
