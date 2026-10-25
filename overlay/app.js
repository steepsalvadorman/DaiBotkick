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
    if (!channel) { notify('Falta el canal. Copia la URL que aparece al conectar con Kick.'); return; }
    const socket = io({ query: { ch: channel }, auth: { token }, reconnectionDelayMax: 10000 });
    let active = false, queue = [], version = 0, playingId = null, pending = false, visible = true;
    let player = null, tiktok = null, retry, generation = 0, apiPromise;
    const tiktokOrigin = 'https://www.tiktok.com';
    // TikTok admite play, pause, seekTo, mute, unMute y changeVolume (0–100).
    const tiktokSend = (type, value) => tiktok?.contentWindow?.postMessage({ type, value, 'x-tiktok-player': true }, tiktokOrigin);
    let musicShown = true, musicVolume = 90;
    let mediaReady = false, paused = false, actuallyPlaying = false;
    // Posición del video actual, para retomarlo si OBS recarga la fuente.
    const resumeKey = `daibot:resume:${channel}`;
    let tiktokTime = 0, resumeAt = 0;
    function readResume() {
        try { return JSON.parse(localStorage.getItem(resumeKey) || 'null'); } catch { return null; }
    }
    function currentTime() {
        if (!mediaReady || !queue[0] || playingId !== queue[0].id) return 0;
        if (queue[0].videoId) return Number(player?.getCurrentTime?.()) || 0;
        if (queue[0].tiktokId) return tiktokTime;
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
        const state = usable && actuallyPlaying && !paused ? 'playing' : usable ? 'paused' : 'idle';
        el('music-transport').setAttribute('data-state', state);
        media.setAttribute('data-state', state);
        const label = paused ? 'Reanudar' : 'Pausar';
        el('music-pause').setAttribute('aria-label', label);
        el('music-pause').setAttribute('title', label);
        el('music-pause-icon').setAttribute('d', paused ? 'M7 4l14 8-14 8Z' : 'M6 5h4v14H6Zm8 0h4v14h-4Z');
        syncIsland();
    }
    let emojiIslandUntil = 0;
    let emojiIslandTimer = 0;
    let islandHover = false;
    let islandPinned = false;
    function syncIsland() {
        const hud = el('hud');
        if (!hud?.classList.contains('dynamic-island')) return;
        const panelsOpen = ['queue', 'volume'].some(name => el(`${name}-panel`) && !el(`${name}-panel`).hidden);
        const musicLive = el('music-transport')?.dataset.state !== 'idle' || queue.length > 0;
        const bingo = document.body.classList.contains('bingo-active');
        const emoji = Date.now() < emojiIslandUntil;
        let mode = 'idle';
        if (islandHover || islandPinned || panelsOpen) mode = 'open';
        else if (bingo) mode = 'bingo';
        else if (emoji) mode = 'emoji';
        else if (musicLive) mode = 'music';
        if (hud.dataset.island !== mode) hud.dataset.island = mode;
    }
    function pokeEmojiIsland() {
        emojiIslandUntil = Date.now() + 8000;
        clearTimeout(emojiIslandTimer);
        emojiIslandTimer = setTimeout(syncIsland, 8000);
        syncIsland();
    }
    async function resumeMedia() {
        paused = false;
        updateTransport();
        if (queue[0]?.videoId) player.playVideo();
        else if (queue[0]?.tiktokId) tiktokSend('play');
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
        else if (queue[0]?.tiktokId) tiktokSend('pause');
        else direct.pause();
        updateTransport();
    });
    el('music-restart').addEventListener('click', () => {
        if (el('music-restart').disabled) return;
        if (queue[0]?.videoId) player.seekTo(0, true);
        else if (queue[0]?.tiktokId) tiktokSend('seekTo', 0);
        else direct.currentTime = 0;
    });
    el('music-stop').addEventListener('click', () => {
        if (el('music-stop').disabled) return;
        paused = true; actuallyPlaying = false;
        if (queue[0]?.videoId) { player.pauseVideo(); player.seekTo(0, true); }
        else if (queue[0]?.tiktokId) { tiktokSend('pause'); tiktokSend('seekTo', 0); }
        else { direct.pause(); direct.currentTime = 0; }
        updateTransport();
    });
    el('music-next').addEventListener('click', () => {
        if (!el('music-next').disabled) finish(playingId);
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
        if (!tiktok) return;
        if (musicVolume === 0) tiktokSend('mute');
        else { tiktokSend('unMute'); tiktokSend('changeVolume', musicVolume); }
    }
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
        tiktok = null; media.removeAttribute('data-source');
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
    addEventListener('message', event => {
        if (event.origin !== tiktokOrigin || !tiktok || event.source !== tiktok.contentWindow) return;
        let data = event.data;
        if (typeof data === 'string') { try { data = JSON.parse(data); } catch { return; } }
        if (!data || typeof data.type !== 'string') return;
        const id = playingId;
        if (data.type === 'onPlayerReady') {
            mediaReady = true; updateTransport();
            applyEmbedVolume();
            if (resumeAt) { tiktokSend('seekTo', resumeAt); resumeAt = 0; }
            if (!paused && !bingoLive()) tiktokSend('play');
        } else if (data.type === 'onCurrentTime') {
            tiktokTime = Number(data.value?.currentTime) || 0;
        } else if (data.type === 'onStateChange') {
            actuallyPlaying = data.value === 1;
            if (actuallyPlaying) { paused = false; notify(''); applyEmbedVolume(); }
            if (data.value === 2) paused = true;
            updateTransport();
            if (data.value === 0) finish(id);
        } else if (data.type === 'onPlayerError') {
            if (data.value?.errorCode === 3002) notify('Audio bloqueado. Interactúa con la fuente de OBS.');
            else finish(id);
        }
    });
    function finish(id) {
        clearResume(id);
        if (!active || pending || playingId !== id || queue[0]?.id !== id || !socket.connected) return;
        pending = true; updateTransport(); socket.emit('advanceQueue', { id, version });
        retry = setTimeout(() => { pending = false; finish(id); }, 5000);
    }
    async function playHead() {
        if (!active || !queue.length || playingId === queue[0].id) return;
        stop(); const item = queue[0]; playingId = item.id; const run = generation;
        const saved = readResume(); tiktokTime = 0;
        resumeAt = saved?.id === item.id && saved.time > 3 ? Math.max(0, saved.time - 2) : 0;
        if (saved && saved.id !== item.id) clearResume(saved.id);
        const title = document.createElement('div'); title.className = 'title-inner marquee';
        title.textContent = `${item.title || 'Sin título'}   ◆   ${item.title || 'Sin título'}   ◆`;
        el('video-title').replaceChildren(title); el('video-title').classList.add('scrolling');
        el('video-requester').textContent = item.user || 'Anónimo'; display();
        media.setAttribute('data-source', item.tiktokId ? 'tiktok' : item.videoId ? 'youtube' : 'direct');
        if (item.tiktokId && /^\d{15,25}$/.test(item.tiktokId)) {
            direct.style.display = 'none'; youtube.style.display = 'block';
            const params = new URLSearchParams({
                autoplay: '1', controls: '0', progress_bar: '0', play_button: '0', volume_control: '0',
                fullscreen_button: '0', timestamp: '0', loop: '0', music_info: '1', description: '0',
                rel: '0', native_context_menu: '0', closed_caption: '0',
            });
            tiktok = document.createElement('iframe');
            tiktok.src = `${tiktokOrigin}/player/v1/${item.tiktokId}?${params}`;
            tiktok.title = item.title || 'Video de TikTok';
            tiktok.allow = 'autoplay; encrypted-media; fullscreen';
            tiktok.referrerPolicy = 'strict-origin-when-cross-origin';
            youtube.append(tiktok);
        } else if (item.videoId) {
            direct.style.display = 'none'; youtube.style.display = 'block';
            try {
                await loadAPI(); if (run !== generation || !active) return;
                const target = document.createElement('div'); youtube.append(target);
                player = new YT.Player(target, {
                    width: '100%', height: '100%', videoId: item.videoId,
                    playerVars: { autoplay: 1, playsinline: 1, controls: 0, rel: 0, origin: location.origin },
                    events: {
                        onReady: event => { if (run === generation) { mediaReady = true; updateTransport(); applyEmbedVolume(); if (resumeAt) { event.target.seekTo(resumeAt, true); resumeAt = 0; } if (!paused && !bingoLive()) event.target.playVideo(); } },
                        onStateChange: event => {
                            if (run !== generation) return;
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
        if (!active) { stop(); stopSpeech(); notify(token ? '' : 'Vista pública. Usa la URL privada de OBS para reproducir.'); }
        else { notify(''); playHead(); } display();
    });
    socket.on('syncQueue', data => {
        if (!Array.isArray(data?.items) || !Number.isSafeInteger(data.version) || data.version < version) return;
        queue = data.items; version = data.version; clearTimeout(retry); pending = false;
        renderQueue();
        if (playingId !== queue[0]?.id) stop(); display(); playHead();
    });
    socket.on('toggleVideo', data => { visible = Boolean(data.showVideo); display(); });
    socket.on('playerAvailable', () => { if (token && !active) { socket.disconnect(); socket.connect(); } });
    socket.on('channelError', message => notify(String(message)));
    socket.on('operationError', message => { clearTimeout(retry); pending = false; updateTransport(); notify(String(message)); });
    socket.on('connect_error', () => notify('Sin conexión. Reintentando...'));
    socket.on('connect', () => { version = 0; notify('Conectado. Sincronizando...'); });
    socket.on('disconnect', reason => {
        bingoState = null; el('bingo-widget').hidden = true; resumeAfterBingo = false;
        document.body.classList.remove('bingo-active');
        active = false; stop(); stopSpeech(); queue = []; display(); notify('Sin conexión. Reintentando...');
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
    const emojiSegments = new Intl.Segmenter('es', { granularity: 'grapheme' });
    const emojiEvents = [];
    let emojiSummary = '';
    function updateEmojiMeter() {
        const cutoff = Date.now() - 300000;
        while (emojiEvents.length && emojiEvents[0].time <= cutoff) emojiEvents.shift();
        const counts = new Map();
        for (const event of emojiEvents) {
            for (const [key, value] of event.counts) counts.set(key, (counts.get(key) || 0) + value);
        }
        const total = [...counts.values()].reduce((sum, count) => sum + count, 0);
        const ranking = [...counts].sort((a, b) => b[1] - a[1]).slice(0, 3);
        // Sin cambios no se reconstruyen los emotes: evita recargar imágenes y repintar cada 5 s.
        const summary = JSON.stringify([total, ranking]);
        if (summary === emojiSummary) return;
        emojiSummary = summary;
        el('emoji-total').textContent = `${total} emojis · 5 min`;
        const top = el('emoji-top'); top.replaceChildren();
        for (const [key, count] of ranking) {
            const entry = document.createElement('span');
            renderChatContent(entry, key);
            entry.append(document.createTextNode(` ×${count}`));
            top.append(entry);
        }
    }
    function countEmojis(content) {
        const counts = new Map();
        const add = key => counts.set(key, (counts.get(key) || 0) + 1);
        const plain = content.replace(/\[emote:(\d{1,20}):([^\]\r\n]{1,100})\]/g, (match) => {
            add(match); return '';
        });
        for (const { segment } of emojiSegments.segment(plain)) {
            if (/\p{Extended_Pictographic}|\p{Regional_Indicator}|\u20e3/u.test(segment)) add(segment);
        }
        if (counts.size) {
            emojiEvents.push({ time: Date.now(), counts });
            pokeEmojiIsland();
        }
        updateEmojiMeter();
    }
    setInterval(updateEmojiMeter, 5000);
    socket.on('chatMessage', data => {
        const item = document.createElement('div'); item.className = 'msg';
        const user = document.createElement('div'); user.className = 'msg-user'; user.textContent = data.user || '';
        const text = document.createElement('div'); text.className = 'msg-text';
        renderChatContent(text, typeof data.content === 'string' ? data.content : '');
        countEmojis(typeof data.content === 'string' ? data.content : '');
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
        else if (queue[0]?.tiktokId) tiktokSend('pause');
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
        if (!active || typeof data?.audioBase64 !== 'string' || !data.audioBase64 || data.audioBase64.length > 6 * 1024 * 1024 || speeches.length >= 32) return;
        speeches.push(data.audioBase64); playSpeech();
    });
    addEventListener('pointerdown', event => {
        if (event.target?.closest('button, input')) return;
        if (active) { if (mediaReady && !paused) resumeMedia(); playSpeech(); }
    });
    updateTransport();
})();
