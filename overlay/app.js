/* GorilinRix: authenticated OBS player and public chat overlay. */
(() => {
    'use strict';
    const el = id => document.getElementById(id);
    const channel = new URLSearchParams(location.search).get('ch') || '';
    if (new URLSearchParams(location.search).get('chat') === '0') document.body.classList.add('no-chat');
    const token = new URLSearchParams(location.hash.slice(1)).get('token') || '';
    const status = document.createElement('div');
    status.id = 'connection-status'; status.setAttribute('role', 'status'); document.body.append(status);
    const notify = text => { status.textContent = text; };
    function updateClock() {
        const now = new Date();
        el('desktop-clock').textContent = new Intl.DateTimeFormat('es', {
            hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
        }).format(now);
        el('desktop-date').textContent = new Intl.DateTimeFormat('es', {
            weekday: 'long', day: 'numeric', month: 'long',
        }).format(now);
    }
    function weatherSummary(code, isDay) {
        if (code === 0) return [isDay ? '☀' : '☾', 'Despejado'];
        if (code === 1) return [isDay ? '🌤' : '☁', 'Mayormente despejado'];
        if (code === 2) return ['⛅', 'Parcialmente nublado'];
        if (code === 3) return ['☁', 'Nublado'];
        if (code === 45 || code === 48) return ['〰', 'Neblina'];
        if (code >= 51 && code <= 57) return ['☂', 'Llovizna'];
        if (code >= 61 && code <= 67) return ['☂', 'Lluvia'];
        if (code >= 71 && code <= 77) return ['❄', 'Nieve'];
        if (code >= 80 && code <= 82) return ['☂', 'Chubascos'];
        if (code === 85 || code === 86) return ['❄', 'Chubascos de nieve'];
        if (code >= 95) return ['ϟ', 'Tormenta'];
        return ['◌', 'Clima variable'];
    }
    async function loadWeather() {
        if (!navigator.geolocation) {
            el('weather-description').textContent = 'Ubicación no disponible';
            el('weather-note').textContent = 'La fuente del navegador no permite geolocalización';
            return;
        }
        el('weather-note').textContent = 'Esperando permiso de ubicación…';
        navigator.geolocation.getCurrentPosition(async position => {
            try {
                const { latitude, longitude } = position.coords;
                const url = `https://api.open-meteo.com/v1/forecast?latitude=${encodeURIComponent(latitude)}&longitude=${encodeURIComponent(longitude)}&current=temperature_2m,weather_code,is_day&timezone=auto`;
                const response = await fetch(url);
                if (!response.ok) throw new Error(`Clima no disponible (${response.status})`);
                const current = (await response.json()).current;
                if (!current || !Number.isFinite(current.temperature_2m) || !Number.isFinite(current.weather_code)) {
                    throw new Error('La respuesta del clima está incompleta');
                }
                const [symbol, description] = weatherSummary(current.weather_code, current.is_day !== 0);
                el('weather-symbol').textContent = symbol;
                el('weather-description').textContent = description;
                el('weather-temperature').textContent = `${Math.round(current.temperature_2m)}°`;
                el('weather-place').textContent = 'TU UBICACIÓN';
                el('weather-note').textContent = 'DATOS ACTUALES · OPEN-METEO';
            } catch (error) {
                el('weather-description').textContent = 'Clima no disponible';
                el('weather-note').textContent = 'Revisa la conexión e inténtalo más tarde';
            }
        }, error => {
            el('weather-description').textContent = 'Ubicación no disponible';
            el('weather-note').textContent = error.code === 1
                ? 'Permite la ubicación en las propiedades del navegador'
                : 'No se pudo determinar la ubicación del dispositivo';
        }, { enableHighAccuracy: false, maximumAge: 600000, timeout: 15000 });
    }
    updateClock();
    loadWeather();
    setInterval(updateClock, 1000);
    setInterval(loadWeather, 1800000);
    function fit() {
        const scale = Math.min(innerWidth / 1920, innerHeight / 1080);
        document.body.style.transform = `scale(${scale})`;
        document.body.style.marginLeft = `${Math.max(0, (innerWidth - 1920 * scale) / 2)}px`;
        document.body.style.marginTop = `${Math.max(0, (innerHeight - 1080 * scale) / 2)}px`;
    }
    fit(); addEventListener('resize', fit);
    if (!channel) { notify('Falta el canal. Copia la URL que aparece al conectar con Kick.'); return; }
    const socket = io({ query: { ch: channel }, auth: { token }, reconnectionDelayMax: 10000 });
    let active = false, queue = [], version = 0, playingId = null, pending = false, visible = true;
    let player = null, timer, retry, generation = 0, apiPromise;
    const media = el('media-widget'), youtube = el('youtube-player'), direct = el('direct-video');
    function stop() {
        generation++; clearTimeout(timer); clearTimeout(retry);
        direct.onended = direct.onerror = null; direct.pause(); direct.removeAttribute('src'); direct.load();
        if (player) { player.destroy(); player = null; }
        youtube.replaceChildren(); playingId = null; pending = false;
    }
    function display() {
        const shown = queue.length && visible && active;
        media.style.visibility = shown ? 'visible' : 'hidden'; media.style.opacity = shown ? '1' : '0';
        media.style.transform = 'scale(1)';
        el('queue-indicator').style.display = queue.length > 1 ? 'inline-block' : 'none';
        el('queue-indicator').textContent = `+${Math.max(0, queue.length - 1)} EN COLA`;
    }
    function loadAPI() {
        if (window.YT?.Player) return Promise.resolve();
        if (apiPromise) return apiPromise;
        apiPromise = new Promise((resolve, reject) => {
            const timeout = setTimeout(() => { script.remove(); reject(new Error('YouTube no responde')); }, 15000);
            window.onYouTubeIframeAPIReady = () => { clearTimeout(timeout); resolve(); };
            const script = document.createElement('script'); script.src = 'https://www.youtube.com/iframe_api';
            script.onerror = () => { clearTimeout(timeout); script.remove(); reject(new Error('No se pudo cargar YouTube')); };
            document.head.append(script);
        }).catch(error => { apiPromise = null; throw error; });
        return apiPromise;
    }
    function finish(id) {
        if (!active || pending || playingId !== id || queue[0]?.id !== id || !socket.connected) return;
        pending = true; socket.emit('advanceQueue', { id, version });
        retry = setTimeout(() => { pending = false; finish(id); }, 5000);
    }
    async function playHead() {
        if (!active || !queue.length || playingId === queue[0].id) return;
        stop(); const item = queue[0]; playingId = item.id; const run = generation;
        const title = document.createElement('div'); title.className = 'title-inner marquee';
        title.textContent = `${item.title || 'Sin título'}   ◆   ${item.title || 'Sin título'}   ◆`;
        el('video-title').replaceChildren(title); el('video-title').classList.add('scrolling');
        el('video-requester').textContent = item.user || 'Anónimo'; display();
        timer = setTimeout(() => finish(item.id), 600000);
        if (item.videoId) {
            direct.style.display = 'none'; youtube.style.display = 'block';
            try {
                await loadAPI(); if (run !== generation || !active) return;
                const target = document.createElement('div'); youtube.append(target);
                player = new YT.Player(target, {
                    width: '100%', height: '100%', videoId: item.videoId,
                    playerVars: { autoplay: 1, playsinline: 1, controls: 0, rel: 0, origin: location.origin },
                    events: {
                        onReady: event => { if (run === generation) { event.target.setVolume(25); event.target.playVideo(); } },
                        onStateChange: event => {
                            if (run !== generation) return;
                            if (event.data === YT.PlayerState.PLAYING) notify('');
                            if (event.data === YT.PlayerState.ENDED) finish(item.id);
                        },
                        onError: () => { if (run === generation) finish(item.id); },
                        onAutoplayBlocked: () => notify('Audio bloqueado. Interactúa con la fuente de OBS.'),
                    },
                });
            } catch (error) { if (run === generation) { notify(error.message); finish(item.id); } }
        } else if (item.url) {
            youtube.style.display = 'none'; direct.style.display = 'block'; direct.src = item.url; direct.volume = 0.25;
            direct.onended = direct.onerror = () => finish(item.id);
            try { await direct.play(); } catch (error) {
                if (run !== generation) return;
                if (error.name === 'NotAllowedError') notify('Audio bloqueado. Interactúa con la fuente de OBS.');
                else finish(item.id);
            }
        } else finish(item.id);
    }
    socket.on('config', config => {
        const name = String(config.channel_name || 'GorilinRix').toUpperCase(); document.title = `${name} — AeroOS`;
        const title = el('stream-title-display'); title.setAttribute('data-text', name);
        const accent = document.createElement('span'); accent.className = 'accent'; const mid = Math.ceil(name.length / 2);
        accent.textContent = name.slice(0, mid); title.replaceChildren(accent, document.createTextNode(name.slice(mid)));
        document.querySelectorAll('.kick-channel-url').forEach(node => { node.textContent = config.kick_url || `kick.com/${channel}`; });
    });
    socket.on('playbackRole', data => {
        active = Boolean(data.active);
        if (!active) { stop(); stopSpeech(); notify(token ? 'Otro overlay controla la reproducción.' : 'Vista pública. Usa la URL privada de OBS para reproducir.'); }
        else { notify(''); playHead(); } display();
    });
    socket.on('syncQueue', data => {
        if (!Array.isArray(data?.items) || !Number.isSafeInteger(data.version) || data.version < version) return;
        queue = data.items; version = data.version; clearTimeout(retry); pending = false;
        if (playingId !== queue[0]?.id) stop(); display(); playHead();
    });
    socket.on('toggleVideo', data => { visible = Boolean(data.showVideo); display(); });
    socket.on('playerAvailable', () => { if (token && !active) { socket.disconnect(); socket.connect(); } });
    socket.on('channelError', message => notify(String(message)));
    socket.on('operationError', message => { clearTimeout(retry); pending = false; notify(String(message)); });
    socket.on('connect_error', () => notify('Sin conexión. Reintentando...'));
    socket.on('connect', () => { version = 0; notify('Conectado. Sincronizando...'); });
    socket.on('disconnect', reason => {
        active = false; stop(); stopSpeech(); queue = []; display(); notify('Sin conexión. Reintentando...');
        if (reason === 'io server disconnect') socket.connect();
    });
    socket.on('chatMessage', data => {
        el('chat-empty').hidden = true;
        const item = document.createElement('div'); item.className = 'msg';
        const user = document.createElement('div'); user.className = 'msg-user'; user.textContent = `▶ ${data.user || ''}`;
        const text = document.createElement('div'); text.className = 'msg-text'; text.textContent = data.content || '';
        item.append(user, text); const body = el('chat-messages'); body.append(item);
        while (body.children.length > 4) body.firstChild.remove();
    });
    let alertTimer;
    const showAlert = data => {
        el('alert-text').textContent = data.message || data.text || '¡Nueva alerta!';
        el('alert-box').classList.add('show'); clearTimeout(alertTimer);
        alertTimer = setTimeout(() => el('alert-box').classList.remove('show'), 6000);
    };
    socket.on('kickAlert', showAlert); socket.on('alert', showAlert);
    const speeches = []; let speech = null, speechTimer;
    function stopSpeech() {
        clearTimeout(speechTimer);
        if (speech) { speech.onended = speech.onerror = null; speech.pause(); speech.src = ''; }
        speech = null; speeches.length = 0;
    }
    function playSpeech() {
        if (speech || !active || !speeches.length) return;
        const audio = new Audio(`data:audio/mp3;base64,${speeches.shift()}`); speech = audio; audio.volume = 1;
        const complete = () => {
            if (speech !== audio) return;
            clearTimeout(speechTimer); audio.onended = audio.onerror = null; audio.pause(); audio.src = '';
            speech = null; playSpeech();
        };
        audio.onended = audio.onerror = complete; speechTimer = setTimeout(complete, 90000);
        audio.play().catch(() => { notify('TTS bloqueado. Interactúa con la fuente de OBS.'); complete(); });
    }
    socket.on('speak', data => {
        if (!active || typeof data.audioBase64 !== 'string' || data.audioBase64.length > 6 * 1024 * 1024 || speeches.length >= 32) return;
        speeches.push(data.audioBase64); playSpeech();
    });
    addEventListener('pointerdown', () => {
        if (active) { player?.playVideo(); if (direct.getAttribute('src')) direct.play().catch(() => {}); playSpeech(); }
    });
})();
