/* DaiBot: authenticated OBS player and public chat overlay. */
(() => {
    'use strict';
    const el = id => document.getElementById(id);
    const channel = new URLSearchParams(location.search).get('ch') || '';
    const avatarOverride = new URLSearchParams(location.search).get('avatar') || '';
    function setChannelAvatar(url) {
        const raw = typeof url === 'string' ? url.trim() : '';
        if (!raw || !/^(\/[a-zA-Z0-9._/-]+|https:\/\/[^\s"'<>]+)$/.test(raw)) return false;
        el('channel-avatar').src = raw;
        document.querySelectorAll('.channel-avatar-sync').forEach(node => { node.src = raw; });
        return true;
    }
    if (avatarOverride) setChannelAvatar(avatarOverride);
    let chatShown = new URLSearchParams(location.search).get('chat') === '1';
    if (!chatShown) document.body.classList.add('no-chat');
    el('chat-toggle').setAttribute('aria-pressed', String(chatShown));
    const token = new URLSearchParams(location.hash.slice(1)).get('token') || '';
    const status = document.createElement('div');
    status.id = 'connection-status'; status.setAttribute('role', 'status'); document.body.append(status);
    const notify = text => { status.textContent = text; };
    // Modo ligero por defecto: sin animaciones decorativas infinitas para no competir con el juego por la GPU.
    // Se desactiva con ?lite=0.
    if (new URLSearchParams(location.search).get('lite') !== '0') document.body.classList.add('lite');
    // Un solo consejo visible. La animación CSS los dejaba los tres en la misma celda y se pintaban juntos.
    const tips = [...document.querySelectorAll('.tip')];
    let tipIndex = 0;
    const showTip = () => tips.forEach((tip, index) => tip.classList.toggle('current', index === tipIndex));
    if (tips.length) {
        showTip();
        if (tips.length > 1) setInterval(() => { tipIndex = (tipIndex + 1) % tips.length; showTip(); }, 9000);
    }
    const clockFormat = new Intl.DateTimeFormat('es', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
    function updateClock() {
        const text = clockFormat.format(new Date());
        // Reescribir el mismo texto cada segundo obliga a repintar sin necesidad.
        if (el('desktop-clock').textContent !== text) el('desktop-clock').textContent = text;
    }
    updateClock();
    setInterval(updateClock, 1000);
    const nativeScale = window.CSS?.supports('zoom', '1') === true;
    function fit() {
        if (innerWidth <= 0 || innerHeight <= 0) return;
        const scale = Math.min(innerWidth / 1920, innerHeight / 1080);
        // Zoom rasterizes text, SVGs and embedded players at the output size.
        document.body.style.zoom = nativeScale ? String(scale) : '1';
        document.body.style.transform = nativeScale ? 'none' : `scale(${scale})`;
        const marginScale = nativeScale ? scale : 1;
        document.body.style.marginLeft = `${Math.max(0, (innerWidth - 1920 * scale) / 2) / marginScale}px`;
        document.body.style.marginTop = `${Math.max(0, (innerHeight - 1080 * scale) / 2) / marginScale}px`;
    }
    fit(); addEventListener('resize', fit);
    // ?demo=1: sin servidor ni canal. Un socket falso simula música, chat, alertas, bingo y luces
    // para diseñar el overlay en local.
    const demo = new URLSearchParams(location.search).has('demo');
    if (!channel && !demo) { notify('Falta el canal. Copia la URL que aparece al conectar con Kick.'); return; }
    const socket = demo ? demoSocket() : io({ query: { ch: channel }, auth: { token }, reconnectionDelayMax: 10000 });
    function demoSocket() {
        const handlers = {};
        const fake = {
            connected: true,
            on(name, fn) { (handlers[name] ||= []).push(fn); return fake; },
            fire(name, data) { (handlers[name] || []).forEach(fn => fn(data)); },
            emit(name, data) {
                if (name === 'playerPing') setTimeout(() => fake.fire('playerPong', { active: true, epoch: data.epoch }), 50);
                if (name === 'advanceQueue') setTimeout(() => fake.onAdvance?.(data.id), 300);
                if (name === 'previousQueue') setTimeout(() => fake.onPrevious?.(), 300);
            },
            connect() {}, disconnect() {},
        };
        return fake;
    }
    let active = false, queue = [], version = 0, playingId = null, pending = false, visible = true;
    let myEpoch = null, pongTimer = null;
    let player = null, retry, generation = 0, apiPromise;
    let musicShown = true, musicVolume = 60, ttsVolume = 100;
    let mediaReady = false, paused = false, actuallyPlaying = false, canGoBack = false;
    // Posición del video actual, para retomarlo si OBS recarga la fuente.
    const resumeKey = `daibot:resume:${channel}`;
    let resumeAt = 0;
    function readResume() {
        try { return JSON.parse(localStorage.getItem(resumeKey) || 'null'); } catch { return null; }
    }
    function currentTime() {
        if (!mediaReady || !queue[0] || playingId !== queue[0].id) return 0;
        if (queue[0].videoId) return Number(player?.getCurrentTime?.()) || 0;
        return Number(direct.currentTime) || 0;
    }
    function saveResume() {
        const time = currentTime();
        if (!active || !playingId || time < 1) return;
        try { localStorage.setItem(resumeKey, JSON.stringify({ id: playingId, time })); } catch {}
    }
    function clearResume(id) {
        if (readResume()?.id !== id) return;
        try { localStorage.removeItem(resumeKey); } catch {}
    }
    setInterval(saveResume, 2000);
    addEventListener('pagehide', saveResume);
    const media = el('media-widget'), youtube = el('youtube-player'), direct = el('direct-video');
    function updateTransport() {
        const usable = Boolean(active && queue.length && playingId === queue[0].id && mediaReady);
        el('music-restart').disabled = !usable || pending;
        el('music-pause').disabled = !usable || pending;
        el('music-stop').disabled = !usable || pending;
        el('music-next').disabled = !active || !queue.length || pending;
        el('music-prev').disabled = !active || !canGoBack || pending;
        const state = usable && actuallyPlaying && !paused ? 'playing' : usable ? 'paused' : 'idle';
        el('music-transport').setAttribute('data-state', state);
        media.setAttribute('data-state', state);
        const label = paused ? 'Reanudar' : 'Pausar';
        el('music-pause').setAttribute('aria-label', label);
        el('music-pause').setAttribute('title', label);
        el('music-pause-icon').setAttribute('d', paused ? 'M7 4l14 8-14 8Z' : 'M6 5h4v14H6Zm8 0h4v14h-4Z');
        syncIsland();
    }
    let islandHover = false;
    let islandPinned = false;
    function syncIsland() {
        const hud = el('hud');
        if (!hud?.classList.contains('dynamic-island')) return;
        const panelsOpen = ['queue', 'volume'].some(name => el(`${name}-panel`) && !el(`${name}-panel`).hidden);
        const musicLive = el('music-transport')?.dataset.state !== 'idle' || queue.length > 0;
        const bingo = document.body.classList.contains('bingo-active');
        let mode = 'idle';
        if (islandHover || islandPinned || panelsOpen) mode = 'open';
        else if (bingo) mode = 'bingo';
        else if (musicLive) mode = 'music';
        if (hud.dataset.island !== mode) { hud.dataset.island = mode; wakeIsland(); }
        if (mode === 'open') wakeIsland();
    }
    // Notch ocultable: tras unos segundos sin interacción se recoge hacia arriba y deja asomar el
    // borde. Vuelve al acercar el ratón a la parte superior, al abrirse o al cambiar lo que muestra.
    const ISLAND_IDLE_MS = 8000;
    let islandTimer;
    function wakeIsland() {
        const hud = el('hud');
        if (!hud?.classList.contains('dynamic-island')) return;
        hud.classList.remove('tucked');
        clearTimeout(islandTimer);
        islandTimer = setTimeout(() => {
            const panelsOpen = ['queue', 'volume'].some(name => el(`${name}-panel`) && !el(`${name}-panel`).hidden);
            if (islandHover || islandPinned || panelsOpen) wakeIsland();
            else hud.classList.add('tucked');
        }, ISLAND_IDLE_MS);
    }
    addEventListener('pointermove', event => {
        const hud = el('hud');
        if (!hud?.getBoundingClientRect) return;
        if (event.clientY <= hud.getBoundingClientRect().bottom + 48) wakeIsland();
    });
    async function resumeMedia() {
        paused = false;
        updateTransport();
        if (queue[0]?.videoId) player.playVideo();
        else {
            const run = generation;
            try { await direct.play(); }
            catch (error) {
                if (run !== generation) return;
                paused = true; actuallyPlaying = false; updateTransport();
                notify(`No se pudo reanudar la música: ${error.message}`);
            }
        }
    }
    el('music-pause').addEventListener('click', () => {
        if (el('music-pause').disabled) return;
        if (paused) { resumeMedia(); return; }
        paused = true; actuallyPlaying = false;
        if (queue[0]?.videoId) player.pauseVideo();
        else direct.pause();
        updateTransport();
    });
    el('music-restart').addEventListener('click', () => {
        if (el('music-restart').disabled) return;
        if (queue[0]?.videoId) player.seekTo(0, true);
        else direct.currentTime = 0;
    });
    el('music-stop').addEventListener('click', () => {
        if (el('music-stop').disabled) return;
        paused = true; actuallyPlaying = false;
        if (queue[0]?.videoId) { player.pauseVideo(); player.seekTo(0, true); }
        else { direct.pause(); direct.currentTime = 0; }
        updateTransport();
    });
    el('music-next').addEventListener('click', () => {
        if (!el('music-next').disabled) finish(playingId);
    });
    // Vuelve a la última canción reproducida; el servidor la devuelve al frente de la cola.
    el('music-prev').addEventListener('click', () => {
        if (el('music-prev').disabled || !socket.connected) return;
        saveResume();
        pending = true; updateTransport(); socket.emit('previousQueue', { version });
        clearTimeout(retry); retry = setTimeout(() => { pending = false; updateTransport(); }, 5000);
    });
    direct.onplaying = () => { if (active && playingId) { actuallyPlaying = true; paused = false; updateTransport(); } };
    direct.onpause = () => { actuallyPlaying = false; updateTransport(); };
    direct.onwaiting = () => { actuallyPlaying = false; updateTransport(); };
    el('chat-toggle').addEventListener('click', () => {
        chatShown = !chatShown;
        document.body.classList.toggle('no-chat', !chatShown);
        el('chat-toggle').setAttribute('aria-pressed', String(chatShown));
    });
    el('music-anchor').addEventListener('click', () => {
        musicShown = !musicShown;
        el('music-anchor').setAttribute('aria-pressed', String(musicShown));
        display();
    });
    function closePanels() {
        for (const name of ['queue', 'volume']) {
            el(`${name}-panel`).hidden = true;
            el(`${name}-toggle`).setAttribute('aria-expanded', 'false');
        }
        syncIsland();
    }
    for (const name of ['queue', 'volume']) {
        el(`${name}-toggle`).addEventListener('click', () => {
            const opened = el(`${name}-toggle`).getAttribute('aria-expanded') === 'true';
            closePanels();
            el(`${name}-panel`).hidden = opened;
            el(`${name}-toggle`).setAttribute('aria-expanded', String(!opened));
            syncIsland();
        });
    }
    el('hud')?.addEventListener('pointerenter', () => { islandHover = true; syncIsland(); });
    el('hud')?.addEventListener('pointerleave', () => { islandHover = false; syncIsland(); });
    el('hud')?.addEventListener('click', event => {
        if (event.target.closest('.island-slot')) return;
        islandPinned = !islandPinned;
        syncIsland();
    });
    addEventListener('keydown', event => { if (event.key === 'Escape') closePanels(); });
    el('music-volume').addEventListener('input', event => {
        const value = Number(event.target.value);
        if (!Number.isFinite(value) || value < 0 || value > 100) {
            notify('Volumen inválido: utiliza un valor entre 0 y 100.');
            return;
        }
        musicVolume = value;
        el('volume-value').textContent = `${value}%`;
        direct.volume = value / 100;
        applyEmbedVolume();
    });
    function applyEmbedVolume() {
        player?.setVolume(musicVolume);
    }
    el('tts-volume').addEventListener('input', event => {
        const value = Number(event.target.value);
        if (!Number.isFinite(value) || value < 0 || value > 100) {
            notify('Volumen inválido: utiliza un valor entre 0 y 100.');
            return;
        }
        ttsVolume = value;
        el('tts-volume-value').textContent = `${value}%`;
        if (speech) speech.volume = value / 100;
    });
    function renderQueue() {
        const list = el('queue-list');
        list.replaceChildren();
        if (!queue.length) {
            const item = document.createElement('li');
            item.textContent = 'La cola está vacía.';
            list.append(item);
        }
        queue.slice(0, 20).forEach((entry, index) => {
            const item = document.createElement('li');
            item.textContent = `${index === 0 ? 'Actual · ' : ''}${entry.title || 'Sin título'} — ${entry.user || 'Anónimo'}`;
            list.append(item);
        });
        if (queue.length > 20) {
            const item = document.createElement('li');
            item.textContent = `Y ${queue.length - 20} solicitudes más.`;
            list.append(item);
        }
        syncIsland();
    }
    renderQueue();
    function stop() {
        mediaReady = false; paused = false; actuallyPlaying = false;
        generation++; clearTimeout(retry);
        direct.onended = direct.onerror = null; direct.pause(); direct.removeAttribute('src'); direct.load();
        if (player) { player.destroy(); player = null; }
        media.removeAttribute('data-source');
        youtube.replaceChildren(); playingId = null; pending = false;
        updateTransport();
    }
    function display() {
        const shown = Boolean(queue.length && visible && active && musicShown && !bingoLive());
        media.setAttribute('data-shown', String(shown));
        media.setAttribute('aria-hidden', String(!shown));
        el('music-anchor').setAttribute('data-playing', String(shown));
        el('topbar-requester').hidden = !(active && queue.length && playingId === queue[0].id);
        el('queue-indicator').style.display = queue.length > 1 ? 'inline-block' : 'none';
        el('queue-indicator').textContent = `+${Math.max(0, queue.length - 1)} en cola`;
        updateTransport();
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
    /** Pide 720p al iframe de YouTube cuando el video lo ofrece; si no, la mejor calidad por debajo. */
    function preferYouTubeQuality(target) {
        if (!target?.getAvailableQualityLevels || !target.setPlaybackQuality) return;
        let levels;
        try { levels = target.getAvailableQualityLevels(); } catch { return; }
        if (!Array.isArray(levels) || !levels.length) return;
        let quality = null;
        if (levels.includes('hd720')) quality = 'hd720';
        else {
            for (const q of ['large', 'medium', 'small', 'tiny']) {
                if (levels.includes(q)) { quality = q; break; }
            }
        }
        if (!quality) return;
        try {
            if (target.getPlaybackQuality?.() !== quality) target.setPlaybackQuality(quality);
        } catch {}
    }
    function finish(id) {
        clearResume(id);
        if (!active || pending || playingId !== id || queue[0]?.id !== id || !socket.connected) return;
        pending = true; updateTransport(); socket.emit('advanceQueue', { id, version });
        retry = setTimeout(() => { pending = false; finish(id); }, 5000);
    }
    async function playHead() {
        if (!active || !queue.length || playingId === queue[0].id) return;
        stop(); const item = queue[0]; playingId = item.id; const run = generation;
        const saved = readResume();
        resumeAt = saved?.id === item.id && saved.time > 3 ? Math.max(0, saved.time - 2) : 0;
        if (saved && saved.id !== item.id) clearResume(saved.id);
        const title = document.createElement('div'); title.className = 'title-inner marquee';
        title.textContent = `${item.title || 'Sin título'}   ◆   ${item.title || 'Sin título'}   ◆`;
        el('video-title').replaceChildren(title); el('video-title').classList.add('scrolling');
        el('video-requester').textContent = item.user || 'Anónimo'; display();
        media.setAttribute('data-source', item.videoId ? 'youtube' : 'direct');
        if (item.videoId) {
            direct.style.display = 'none'; youtube.style.display = 'block';
            try {
                await loadAPI(); if (run !== generation || !active) return;
                const target = document.createElement('div'); youtube.append(target);
                player = new YT.Player(target, {
                    width: '100%', height: '100%', videoId: item.videoId,
                    playerVars: { autoplay: 1, playsinline: 1, controls: 0, rel: 0, origin: location.origin },
                    events: {
                        onReady: event => {
                            if (run !== generation) return;
                            mediaReady = true;
                            updateTransport();
                            applyEmbedVolume();
                            preferYouTubeQuality(event.target);
                            if (resumeAt) { event.target.seekTo(resumeAt, true); resumeAt = 0; }
                            if (!paused && !bingoLive()) event.target.playVideo();
                        },
                        onStateChange: event => {
                            if (run !== generation) return;
                            if (event.data === YT.PlayerState.BUFFERING || event.data === YT.PlayerState.PLAYING) {
                                preferYouTubeQuality(event.target);
                            }
                            actuallyPlaying = event.data === YT.PlayerState.PLAYING;
                            if (actuallyPlaying) { paused = false; notify(''); applyEmbedVolume(); }
                            if (event.data === YT.PlayerState.PAUSED) paused = true;
                            updateTransport();
                            if (event.data === YT.PlayerState.ENDED) finish(item.id);
                        },
                        onError: () => { if (run === generation) finish(item.id); },
                        onAutoplayBlocked: () => notify('Audio bloqueado. Interactúa con la fuente de OBS.'),
                    },
                });
            } catch (error) { if (run === generation) { notify(error.message); finish(item.id); } }
        } else if (item.url) {
            youtube.style.display = 'none'; direct.style.display = 'block'; direct.src = item.url; direct.volume = musicVolume / 100;
            if (resumeAt) { const at = resumeAt; resumeAt = 0; direct.onloadedmetadata = () => { direct.onloadedmetadata = null; direct.currentTime = at; }; }
            mediaReady = true; updateTransport();
            direct.onended = direct.onerror = () => finish(item.id);
            if (bingoLive()) paused = true;
            else try { await direct.play(); } catch (error) {
                if (run !== generation) return;
                if (error.name === 'NotAllowedError') notify('Audio bloqueado. Interactúa con la fuente de OBS.');
                else finish(item.id);
            }
        } else finish(item.id);
    }
    socket.on('config', config => {
        // Nombre tal cual en Kick: conserva mayúsculas y guiones bajos (Frank_Arr).
        const name = String(config.channel_name || channel || 'seniordai');
        document.title = name;
        el('stream-title-display').textContent = name;
        if (!avatarOverride && setChannelAvatar(config.avatar_url || config.avatar)) el('channel-avatar').alt = `Avatar de ${name}`;
        document.querySelectorAll('.kick-channel-url').forEach(node => { node.textContent = config.kick_url || `kick.com/${channel}`; });
    });
    socket.on('playbackRole', data => {
        active = Boolean(data.active);
        myEpoch = active ? data.epoch ?? null : null;
        if (!active) { stop(); stopSpeech(); notify(token ? '' : 'Vista pública. Usa la URL privada de OBS para reproducir.'); }
        else { notify(''); playHead(); } display();
    });
    // Relevo del slot: si otra página lo tomó, el pong avisa y cortamos el audio enseguida;
    // sin pong en 10s la conexión está caída aunque el socket crea estar vivo (half-open).
    function losePlayback() {
        if (!active) return;
        active = false; myEpoch = null; stop(); stopSpeech(); display();
        socket.disconnect(); socket.connect();
    }
    setInterval(() => {
        if (!active || !socket.connected || myEpoch == null || pongTimer) return;
        socket.emit('playerPing', { epoch: myEpoch });
        pongTimer = setTimeout(losePlayback, 10000);
    }, 4000);
    socket.on('playerPong', data => {
        clearTimeout(pongTimer); pongTimer = null;
        if (!data?.active || data.epoch !== myEpoch) losePlayback();
    });
    socket.on('syncQueue', data => {
        if (!Array.isArray(data?.items) || !Number.isSafeInteger(data.version) || data.version < version) return;
        queue = data.items; version = data.version; clearTimeout(retry); pending = false;
        canGoBack = data.previous === true;
        renderQueue();
        if (playingId !== queue[0]?.id) stop(); display(); playHead();
    });
    socket.on('toggleVideo', data => { visible = Boolean(data.showVideo); display(); });
    // Luces de fiesta sobre todo el overlay: el streamer las enciende o apaga con !disco.
    const stageLights = document.createElement('div');
    stageLights.className = 'stage-lights'; stageLights.setAttribute('aria-hidden', 'true');
    const lightParts = ['wash wash-1', 'wash wash-2', 'wash wash-3', 'pulse',
        ...[1, 2, 3, 4, 5, 6].map(n => `beam beam-${n}`), ...[1, 2, 3, 4].map(n => `laser laser-${n}`),
        'garland garland-top', 'garland garland-bottom'];
    for (const part of lightParts) {
        const layer = document.createElement('span'); layer.className = part;
        // Cada guirnalda tiene tres tiras de focos que se encienden en cadena.
        if (part.startsWith('garland')) for (let i = 0; i < 3; i++) layer.append(document.createElement('i'));
        stageLights.append(layer);
    }
    document.body.append(stageLights);
    socket.on('stageLights', data => { document.body.classList.toggle('stage-lights-on', data?.on === true); });
    socket.on('playerAvailable', () => { if (token && !active) { socket.disconnect(); socket.connect(); } });
    socket.on('channelError', message => notify(String(message)));
    socket.on('operationError', message => { clearTimeout(retry); pending = false; updateTransport(); notify(String(message)); });
    socket.on('connect_error', () => notify('Sin conexión. Reintentando...'));
    socket.on('connect', () => { version = 0; notify('Conectado. Sincronizando...'); });
    socket.on('disconnect', reason => {
        bingoState = null; el('bingo-widget').hidden = true; resumeAfterBingo = false;
        document.body.classList.remove('bingo-active');
        active = false; myEpoch = null; clearTimeout(pongTimer); pongTimer = null;
        stop(); stopSpeech(); queue = []; canGoBack = false; display(); notify('Sin conexión. Reintentando...');
        renderQueue(); closePanels();
        if (reason === 'io server disconnect') socket.connect();
    });
    function renderChatContent(target, content) {
        const pattern = /\[emote:(\d{1,20}):([^\]\r\n]{1,100})\]/g;
        let offset = 0;
        for (const match of content.matchAll(pattern)) {
            target.append(document.createTextNode(content.slice(offset, match.index)));
            const image = document.createElement('img');
            image.className = 'chat-emote';
            image.alt = match[2]; image.title = match[2];
            image.src = `https://files.kick.com/emotes/${match[1]}/fullsize`;
            image.referrerPolicy = 'no-referrer';
            image.addEventListener('error', () => {
                image.replaceWith(document.createTextNode(`:${match[2]}:`));
                notify(`No se pudo cargar el emote ${match[2]} de Kick.`);
            }, { once: true });
            target.append(image);
            offset = match.index + match[0].length;
        }
        if (offset === 0) target.textContent = content;
        else target.append(document.createTextNode(content.slice(offset)));
    }
    socket.on('chatMessage', data => {
        const item = document.createElement('div'); item.className = 'msg';
        const user = document.createElement('div'); user.className = 'msg-user'; user.textContent = data.user || '';
        const text = document.createElement('div'); text.className = 'msg-text';
        const content = typeof data.content === 'string' ? data.content : '';
        // Un usuario baneado solo comenta texto: sin emotes ni estilo destacado.
        if (data.textOnly === true) { item.className = 'msg text-only'; text.textContent = content; }
        else renderChatContent(text, content);
        item.append(user, text); const body = el('chat-messages'); body.append(item);
        while (body.children.length > 4) body.firstChild.remove();
    });
    let alertTimer;
    // El backend marca cada alerta con su tipo; el tema le da título y color propios.
    const alertTitles = { follow: 'Nuevo seguidor', sub: 'Nuevo suscriptor', giftsub: 'Suscripciones regaladas' };
    const showAlert = data => {
        const type = Object.hasOwn(alertTitles, data?.type) ? data.type : 'other';
        el('alert-box').setAttribute('data-type', type);
        if (el('alert-title')) el('alert-title').textContent = alertTitles[type] || 'Nueva alerta';
        el('alert-text').textContent = data.message || data.text || '¡Nueva alerta!';
        el('alert-box').classList.add('show'); clearTimeout(alertTimer);
        alertTimer = setTimeout(() => el('alert-box').classList.remove('show'), 6000);
        if (type === 'follow' || type === 'sub') playJingle();
    };
    socket.on('kickAlert', showAlert); socket.on('alert', showAlert);
    // Sintonía de follow y suscripción. Suena solo en el reproductor activo (OBS) para no duplicar
    // audio, con el volumen de "Voces". Usa /sounds/alert.mp3 si existe; si no, un chime sintetizado.
    // Mientras suena, el TTS de la alerta espera su turno.
    let jingle = null, jingleFile = true, audioContext = null;
    function jingleDone(token) {
        if (jingle !== token) return;
        jingle = null; playSpeech();
    }
    function playJingle() {
        if (!active || jingle) return;
        if (!jingleFile) { synthJingle(); return; }
        const audio = new Audio('/sounds/alert.mp3'); jingle = audio; audio.volume = ttsVolume / 100;
        const fallback = () => { if (jingle !== audio) return; jingleFile = false; jingle = null; synthJingle(); };
        audio.onended = () => jingleDone(audio);
        audio.onerror = fallback;
        audio.play().catch(error => {
            if (error?.name === 'NotAllowedError') { notify('Sonido bloqueado. Interactúa con la fuente de OBS.'); jingleDone(audio); }
            else fallback();
        });
        setTimeout(() => jingleDone(audio), 15000);
    }
    /** Arpegio ascendente con brillo de cristal, propio de este overlay. */
    function synthJingle() {
        const Context = window.AudioContext || window.webkitAudioContext;
        if (!Context) return;
        audioContext ||= new Context();
        audioContext.resume?.();
        const token = {}; jingle = token;
        const now = audioContext.currentTime + 0.05;
        const master = audioContext.createGain();
        master.gain.value = 0.35 * (ttsVolume / 100);
        master.connect(audioContext.destination);
        [523.25, 659.25, 783.99, 1046.5, 1318.5].forEach((freq, index) => {
            const start = now + index * 0.11;
            for (const [type, detune, level] of [['triangle', 0, 1], ['sine', 1200, 0.25]]) {
                const osc = audioContext.createOscillator(), gain = audioContext.createGain();
                osc.type = type; osc.frequency.value = freq; osc.detune.value = detune;
                gain.gain.setValueAtTime(0, start);
                gain.gain.linearRampToValueAtTime(level, start + 0.02);
                gain.gain.exponentialRampToValueAtTime(0.001, start + 0.9);
                osc.connect(gain).connect(master);
                osc.start(start); osc.stop(start + 1);
            }
        });
        setTimeout(() => jingleDone(token), 1500);
    }
    let bingoState = null, bingoShown = true, resumeAfterBingo = false;
    function bingoLive() { return document.body.classList.contains('bingo-active'); }
    function syncBingoStage() {
        const live = Boolean(bingoShown && bingoState && bingoState.phase !== 'idle');
        document.body.classList.toggle('bingo-active', live);
        syncIsland();
        if (!live) {
            if (resumeAfterBingo && active && queue.length) { resumeAfterBingo = false; resumeMedia(); }
            return;
        }
        if (actuallyPlaying && !paused) resumeAfterBingo = true;
        if (!active || !queue.length || paused) return;
        paused = true; actuallyPlaying = false;
        if (queue[0]?.videoId) player?.pauseVideo();
        else direct.pause();
        updateTransport();
    }
    const bingoLabel = number => `${'BINGO'[Math.floor((number - 1) / 15)]}-${number}`;
    const bingoCells = [];
    for (let group = 0; group < 5; group++) {
        const row = document.createElement('div'); row.className = 'bingo-board-row';
        const label = document.createElement('b'); label.textContent = 'BINGO'[group]; row.append(label);
        for (let n = group * 15 + 1; n <= group * 15 + 15; n++) {
            const cell = document.createElement('span'); cell.textContent = String(n);
            row.append(cell); bingoCells[n] = cell;
        }
        el('bingo-board').append(row);
    }
    el('bingo-toggle').addEventListener('click', () => {
        bingoShown = !bingoShown;
        el('bingo-toggle').setAttribute('aria-pressed', String(bingoShown));
        el('bingo-widget').hidden = !bingoShown || !bingoState || bingoState.phase === 'idle';
        syncBingoStage();
    });
    socket.on('bingoState', data => {
        const number = n => Number.isInteger(n) && n >= 1 && n <= 75;
        if (!data || !Number.isSafeInteger(data.round) || data.round < 0 ||
            !['idle', 'open', 'running', 'exhausted', 'won'].includes(data.phase) ||
            !Array.isArray(data.drawn) || data.drawn.length > 75 || !data.drawn.every(number) ||
            new Set(data.drawn).size !== data.drawn.length || !Number.isInteger(data.participants) ||
            data.participants < 0 || data.participants > 1000 ||
            (data.phase === 'won' && (typeof data.winner !== 'string' || !Array.isArray(data.card) ||
                data.card.length !== 25 || !data.card.every(n => n === 0 || number(n)) ||
                !Array.isArray(data.line) || data.line.length !== 5 || !data.line.every(n => n === 0 || data.drawn.includes(n))))) {
            notify('No se pudo mostrar el bingo: estado inválido del servidor.');
            return;
        }
        const previous = bingoState;
        bingoState = data;
        const widget = el('bingo-widget');
        widget.hidden = !bingoShown || data.phase === 'idle';
        syncBingoStage();
        widget.setAttribute('data-phase', data.phase);
        el('bingo-count').textContent = `${data.drawn.length} / 75`;
        el('bingo-participants').textContent = `${data.participants} cartones`;
        el('bingo-status').textContent = data.phase === 'won' ? `¡Bingo! ${data.winner}` :
            ({ idle: 'Esperando partida', open: 'Inscripciones abiertas', running: 'Una bola cada 10 s', exhausted: 'Todas las bolas sorteadas' })[data.phase];
        el('bingo-help').textContent = data.phase === 'won' ? 'Línea verificada en el servidor' :
            data.phase === 'open' ? 'Recibe tu cartón con !carton' : 'Fila, columna o diagonal: !bingo';
        const latest = data.drawn.at(-1);
        const ball = el('bingo-ball');
        ball.textContent = latest ? bingoLabel(latest) : '—';
        if (latest && (previous?.round !== data.round || previous?.drawn.at(-1) !== latest)) {
            ball.style.animation = 'none'; void ball.offsetWidth; ball.style.animation = '';
            ball.classList.add('bingo-ball-arrival');
        }
        for (let n = 1; n <= 75; n++) {
            bingoCells[n].classList.toggle('called', data.drawn.includes(n));
            bingoCells[n].classList.toggle('latest', n === latest);
            bingoCells[n].setAttribute('aria-label', `${bingoLabel(n)}${data.drawn.includes(n) ? ', sorteado' : ', pendiente'}`);
        }
        el('bingo-history').textContent = data.drawn.length ? `Últimas: ${data.drawn.slice(-5).map(bingoLabel).join(' · ')}` : '75 bolas · Sin repeticiones · Centro libre';
        const card = el('bingo-winning-card'); card.replaceChildren(); card.hidden = data.phase !== 'won';
        if (data.phase === 'won') {
            for (const letter of 'BINGO') {
                const heading = document.createElement('b'); heading.textContent = letter; card.append(heading);
            }
            data.card.forEach(n => {
                const cell = document.createElement('span'); cell.textContent = n === 0 ? '★' : String(n);
                cell.classList.toggle('winning', data.line.includes(n));
                card.append(cell);
            });
        }
    });
    const speeches = []; let speech = null, speechTimer;
    function stopSpeech() {
        clearTimeout(speechTimer);
        if (speech) { speech.onended = speech.onerror = null; speech.pause(); speech.src = ''; }
        speech = null; speeches.length = 0;
        if (jingle?.pause) { jingle.onended = jingle.onerror = null; jingle.pause(); }
        jingle = null;
    }
    function playSpeech() {
        if (speech || jingle || !active || !speeches.length) return;
        const audio = new Audio(`data:audio/mp3;base64,${speeches.shift()}`); speech = audio; audio.volume = ttsVolume / 100;
        const complete = () => {
            if (speech !== audio) return;
            clearTimeout(speechTimer); audio.onended = audio.onerror = null; audio.pause(); audio.src = '';
            speech = null; playSpeech();
        };
        audio.onended = audio.onerror = complete; speechTimer = setTimeout(complete, 90000);
        audio.play().catch(() => { notify('TTS bloqueado. Interactúa con la fuente de OBS.'); complete(); });
    }
    socket.on('speak', data => {
        if (!active || typeof data?.audioBase64 !== 'string' || !data.audioBase64 || data.audioBase64.length > 6 * 1024 * 1024 || speeches.length >= 32) return;
        speeches.push(data.audioBase64); playSpeech();
    });
    addEventListener('pointerdown', event => {
        if (event.target?.closest('button, input')) return;
        if (active) { if (mediaReady && !paused) resumeMedia(); playSpeech(); }
    });
    updateTransport();
    if (demo) startDemo();
    // Guion del modo demo. Teclas: 1 alerta · 2 chat · 3 bingo (avanza fase) · 4 luces · 5 añade canción.
    function startDemo() {
        const songs = [
            { title: 'lofi hip hop radio — beats to relax/study to', videoId: 'jfKfPfyJRdk' },
            { title: 'synthwave radio — beats to chill/game to', videoId: '4xDzrJKXOOY' },
        ];
        const users = ['Frank_Arr', 'dalia', 'jorge', 'alex', 'ninja_volt', 'saturnita'];
        const lines = ['¡Buenas, chat!', 'qué temazo 🔥', '!play daft punk', 'ese overlay está brutal',
            'GG', 'saludos desde Lima', '[emote:39261:KEKW] jajaja', 'sube el volumen porfa'];
        const pick = list => list[Math.floor(Math.random() * list.length)];
        let version = 1, nextId = 1, items = [], history = [];
        const addSong = () => { const song = songs[(nextId - 1) % songs.length]; items.push({ id: `demo-${nextId++}`, user: pick(users), ...song }); };
        const sync = () => socket.fire('syncQueue', { items: [...items], version: version++, previous: history.length > 0 });
        socket.onAdvance = id => {
            const played = items.find(item => item.id === id);
            if (played) history.push(played);
            items = items.filter(item => item.id !== id);
            if (!items.length) addSong();
            sync();
        };
        socket.onPrevious = () => {
            if (history.length) items.unshift(history.pop());
            sync();
        };
        const chat = () => socket.fire('chatMessage', { user: pick(users), content: pick(lines), textOnly: Math.random() < 0.1 });
        // Mismos datos que envía webhook.rs para follow, sub y giftsub.
        const alertKinds = [
            user => ({ type: 'follow', username: user, message: `¡Gracias por el follow, ${user}!` }),
            user => ({ type: 'sub', username: user, months: 3, message: `¡${user} se suscribió por 3 meses!` }),
            user => ({ type: 'giftsub', message: `¡${user} regaló 5 suscripciones!` }),
        ];
        let alertIndex = 0;
        const alert = () => socket.fire('kickAlert', alertKinds[alertIndex++ % alertKinds.length](pick(users)));
        let lights = false;
        const toggleLights = () => { lights = !lights; socket.fire('stageLights', { on: lights }); };
        // Bingo: idle → open → running (bola cada 2 s) → won → idle.
        const card = Array.from({ length: 25 }, (_, i) => (i === 12 ? 0 : (i % 5) * 15 + Math.floor(i / 5) * 3 + 1));
        const line = card.slice(0, 5);
        let bingo = { round: 0, phase: 'idle', drawn: [], participants: 0 }, drawTimer;
        const sendBingo = () => socket.fire('bingoState', { ...bingo, drawn: [...bingo.drawn] });
        const draw = () => {
            const left = Array.from({ length: 75 }, (_, i) => i + 1).filter(n => !bingo.drawn.includes(n) && !line.includes(n));
            if (left.length) bingo.drawn.push(pick(left));
            sendBingo();
        };
        const nextBingo = () => {
            clearInterval(drawTimer);
            const phase = { idle: 'open', open: 'running', running: 'won', won: 'idle' }[bingo.phase] || 'idle';
            if (phase === 'open') bingo = { round: bingo.round + 1, phase, drawn: [], participants: 12 };
            else if (phase === 'running') { bingo.phase = phase; drawTimer = setInterval(draw, 2000); }
            else if (phase === 'won') {
                for (const n of line) if (n && !bingo.drawn.includes(n)) bingo.drawn.push(n);
                bingo = { ...bingo, phase, winner: pick(users), card, line };
            } else bingo = { round: bingo.round, phase, drawn: [], participants: 0 };
            sendBingo();
        };
        socket.fire('config', { channel_name: channel || 'seniordai', kick_url: `kick.com/${channel || 'seniordai'}` });
        socket.fire('playbackRole', { active: true, epoch: 1 });
        addSong(); addSong(); sync();
        socket.fire('bingoState', bingo);
        chat(); chat();
        setInterval(chat, 4000);
        setInterval(alert, 25000);
        setTimeout(alert, 3000);
        addEventListener('keydown', event => {
            if (event.target?.closest?.('input')) return;
            const action = { 1: alert, 2: chat, 3: nextBingo, 4: toggleLights, 5: () => { addSong(); sync(); } }[event.key];
            if (action) { event.preventDefault(); action(); }
        });
        notify('Modo demo · 1 alerta · 2 chat · 3 bingo · 4 luces · 5 canción');
        setTimeout(() => notify(''), 8000);
    }
})();
