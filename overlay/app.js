/* GorilinRix: authenticated OBS player and public chat overlay. */
(() => {
    'use strict';
    const el = id => document.getElementById(id);
    const channel = new URLSearchParams(location.search).get('ch') || '';
    let chatShown = new URLSearchParams(location.search).get('chat') !== '0';
    if (!chatShown) document.body.classList.add('no-chat');
    el('chat-toggle').setAttribute('aria-pressed', String(chatShown));
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
    updateClock();
    setInterval(updateClock, 1000);
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
    let musicShown = true, musicVolume = 25;
    let mediaReady = false, paused = false, actuallyPlaying = false;
    const media = el('media-widget'), youtube = el('youtube-player'), direct = el('direct-video');
    function updateTransport() {
        const usable = Boolean(active && queue.length && playingId === queue[0].id && mediaReady);
        el('music-restart').disabled = !usable || pending;
        el('music-pause').disabled = !usable || pending;
        el('music-next').disabled = !active || !queue.length || pending;
        el('music-transport').setAttribute('data-state', usable && actuallyPlaying && !paused ? 'playing' : usable ? 'paused' : 'idle');
        const label = paused ? 'Reanudar' : 'Pausar';
        el('music-pause').setAttribute('aria-label', label);
        el('music-pause').setAttribute('title', label);
        el('music-pause-icon').setAttribute('d', paused ? 'M7 4l14 8-14 8Z' : 'M6 5h4v14H6Zm8 0h4v14h-4Z');
    }
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
        if (queue[0]?.videoId) player.pauseVideo(); else direct.pause();
        updateTransport();
    });
    el('music-restart').addEventListener('click', () => {
        if (el('music-restart').disabled) return;
        if (queue[0]?.videoId) player.seekTo(0, true); else direct.currentTime = 0;
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
    }
    for (const name of ['queue', 'volume']) {
        el(`${name}-toggle`).addEventListener('click', () => {
            const opened = el(`${name}-toggle`).getAttribute('aria-expanded') === 'true';
            closePanels();
            el(`${name}-panel`).hidden = opened;
            el(`${name}-toggle`).setAttribute('aria-expanded', String(!opened));
        });
    }
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
        player?.setVolume(value);
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
    }
    renderQueue();
    function stop() {
        mediaReady = false; paused = false; actuallyPlaying = false;
        generation++; clearTimeout(timer); clearTimeout(retry);
        direct.onended = direct.onerror = null; direct.pause(); direct.removeAttribute('src'); direct.load();
        if (player) { player.destroy(); player = null; }
        youtube.replaceChildren(); playingId = null; pending = false;
        updateTransport();
    }
    function display() {
        const shown = Boolean(queue.length && visible && active && musicShown);
        media.setAttribute('data-shown', String(shown));
        media.setAttribute('aria-hidden', String(!shown));
        el('music-anchor').setAttribute('data-playing', String(shown));
        el('queue-indicator').style.display = queue.length > 1 ? 'inline-block' : 'none';
        el('queue-indicator').textContent = `+${Math.max(0, queue.length - 1)} EN COLA`;
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
    function finish(id) {
        if (!active || pending || playingId !== id || queue[0]?.id !== id || !socket.connected) return;
        pending = true; updateTransport(); socket.emit('advanceQueue', { id, version });
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
                        onReady: event => { if (run === generation) { mediaReady = true; updateTransport(); event.target.setVolume(musicVolume); event.target.playVideo(); } },
                        onStateChange: event => {
                            if (run !== generation) return;
                            actuallyPlaying = event.data === YT.PlayerState.PLAYING;
                            if (actuallyPlaying) { paused = false; notify(''); }
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
            mediaReady = true; updateTransport();
            direct.onended = direct.onerror = () => finish(item.id);
            try { await direct.play(); } catch (error) {
                if (run !== generation) return;
                if (error.name === 'NotAllowedError') notify('Audio bloqueado. Interactúa con la fuente de OBS.');
                else finish(item.id);
            }
        } else finish(item.id);
    }
    socket.on('config', config => {
        const name = String(config.channel_name || channel || 'seniordai').toLowerCase();
        document.title = name;
        el('stream-title-display').textContent = name;
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
        bingoState = null; el('bingo-widget').hidden = true;
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
    function updateEmojiMeter() {
        const cutoff = Date.now() - 300000;
        while (emojiEvents.length && emojiEvents[0].time <= cutoff) emojiEvents.shift();
        const counts = new Map();
        for (const event of emojiEvents) {
            for (const [key, value] of event.counts) counts.set(key, (counts.get(key) || 0) + value);
        }
        const total = [...counts.values()].reduce((sum, count) => sum + count, 0);
        el('emoji-total').textContent = `${total} emojis · 5 min`;
        const top = el('emoji-top'); top.replaceChildren();
        for (const [key, count] of [...counts].sort((a, b) => b[1] - a[1]).slice(0, 3)) {
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
        if (counts.size) emojiEvents.push({ time: Date.now(), counts });
        updateEmojiMeter();
    }
    setInterval(updateEmojiMeter, 5000);
    socket.on('chatMessage', data => {
        el('chat-empty').hidden = true;
        const item = document.createElement('div'); item.className = 'msg';
        const user = document.createElement('div'); user.className = 'msg-user'; user.textContent = `▶ ${data.user || ''}`;
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
    let bingoState = null, bingoShown = true;
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
        if (!active || typeof data.audioBase64 !== 'string' || data.audioBase64.length > 6 * 1024 * 1024 || speeches.length >= 32) return;
        speeches.push(data.audioBase64); playSpeech();
    });
    addEventListener('pointerdown', event => {
        if (event.target?.closest('button, input')) return;
        if (active) { if (mediaReady && !paused) resumeMedia(); playSpeech(); }
    });
    updateTransport();
})();
