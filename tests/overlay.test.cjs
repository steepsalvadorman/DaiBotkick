const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function overlay(hash = '#token=private', search = '?ch=alpha', options = {}) {
    class Element {
        constructor() { this.children = []; this.listeners = {}; this.style = { setProperty(name, value) { this[name] = value; } }; this.textContent = ''; this.attributes = {}; const classes = new Set(); this.classList = { add(...names) { names.forEach(name => classes.add(name)); }, remove(...names) { names.forEach(name => classes.delete(name)); }, toggle(name, force) { if (force) classes.add(name); else classes.delete(name); }, contains(name) { return classes.has(name); } }; }
        addEventListener(name, callback) { this.listeners[name] = callback; }
        append(...nodes) { for (const node of nodes) { node.parent = this; this.children.push(node); } }
        replaceChildren(...nodes) { this.children = []; this.append(...nodes); }
        get firstChild() { return this.children[0]; }
        remove() { this.parent.children.splice(this.parent.children.indexOf(this), 1); }
        setAttribute(name, value) { this.attributes[name] = value; }
        getAttribute(name) { return this.attributes[name]; }
        removeAttribute(name) { delete this.attributes[name]; if (name === 'src') this.src = ''; }
        pause() { this.paused = true; }
        load() {}
        play() { this.paused = false; return options.playError ? Promise.reject(options.playError) : Promise.resolve(); }
        set innerHTML(_) { throw new Error('Unsafe HTML insertion'); }
    }
    const elements = new Map();
    const get = id => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); };
    const handlers = {}, emitted = [], audios = [], players = [], calls = [];
    let timerId = 0, intervalCallback;
    const timers = new Map();
    const socket = { connected: true, on(name, callback) { handlers[name] = callback; }, emit(...args) { emitted.push(args); }, disconnect() { calls.push('disconnect'); }, connect() { calls.push('connect'); } };
    const context = {
        document: { body: new Element(), head: new Element(), getElementById: get, createElement: () => new Element(), createTextNode: text => ({ textContent: text }), querySelectorAll: () => [] },
        location: { hash, search, origin: 'https://bot.example' },
        innerWidth: 1920, innerHeight: 1080, URLSearchParams, Date, Promise,
        io: options => { context.options = options; return socket; }, addEventListener() {},
        setTimeout(fn) { timers.set(++timerId, fn); return timerId; }, clearTimeout(id) { timers.delete(id); }, setInterval(fn, delay) { (context.intervals ||= []).push({ fn, delay }); intervalCallback = fn; },
        Audio: class extends Element { constructor(src) { super(); this.src = src; audios.push(this); } },
        YT: { PlayerState: { PLAYING: 1, PAUSED: 2, ENDED: 0 }, Player: class {
            constructor(target, options) { this.options = options; players.push(this); }
            destroy() { this.destroyed = true; } playVideo() { this.paused = false; } pauseVideo() { this.paused = true; } seekTo(time) { this.time = time; } setVolume() {}
        } },
    };
    context.window = context;
    vm.runInNewContext(fs.readFileSync(__dirname + '/../overlay/app.js', 'utf8'), context);
    return { handlers, emitted, audios, players, get, context, timers, calls, socket, intervalCallback: () => intervalCallback };
}
const item = (id, url = 'https://cdn.example/video.mp4') => ({ id, title: '<img src=x onerror=alert(1)>', user: 'viewer', url });

test('music transport pauses, resumes, restarts and skips direct media safely', async () => {
    const app = overlay();
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    const video = app.get('direct-video');
    video.onplaying();
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'playing');
    app.get('music-pause').listeners.click();
    assert.equal(video.paused, true);
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'paused');
    video.currentTime = 40;
    app.get('music-restart').listeners.click();
    assert.equal(video.currentTime, 0);
    assert.equal(video.paused, true);
    app.get('music-pause').listeners.click();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(video.paused, false);
    video.onplaying();
    app.get('music-next').listeners.click();
    app.get('music-next').listeners.click();
    assert.equal(app.emitted.length, 1);
    assert.equal(app.emitted[0][1].id, 'one');
    app.handlers.disconnect('transport close');
    assert.equal(app.get('music-pause').disabled, true);
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'idle');
});
test('YouTube transport uses player methods and public transport remains disabled', async () => {
    const app = overlay();
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [{ ...item('yt'), videoId: 'dQw4w9WgXcQ' }], version: 1 });
    await new Promise(resolve => setImmediate(resolve));
    const player = app.players[0];
    player.options.events.onReady({ target: player });
    player.options.events.onStateChange({ data: 1 });
    app.get('music-pause').listeners.click();
    assert.equal(player.paused, true);
    app.get('music-restart').listeners.click();
    assert.equal(player.time, 0);
    app.get('music-pause').listeners.click();
    assert.equal(player.paused, false);
    const publicApp = overlay('');
    publicApp.handlers.syncQueue({ items: [item('one')], version: 1 });
    publicApp.get('music-next').listeners.click();
    assert.equal(publicApp.emitted.length, 0);
    assert.equal(publicApp.get('music-pause').disabled, true);
});
test('external chat, config and titles are rendered as literal text', () => {
    const app = overlay();
    app.handlers.config({ channel_name: '<script>', kick_url: 'kick.com/a' });
    app.handlers.chatMessage({ user: '<img>', content: '<script>alert(1)</script>' });
    assert.equal(app.get('chat-messages').children[0].children[1].textContent, '<script>alert(1)</script>');
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.match(app.get('video-title').children[0].textContent, /<img/);
});
test('topbar controls toggle local panels, chat and player without changing queue permissions', () => {
    const app = overlay('#token=private', '?ch=alpha&chat=0');
    assert.equal(app.get('chat-toggle').getAttribute('aria-pressed'), 'false');
    app.get('chat-toggle').listeners.click();
    assert.equal(app.get('chat-toggle').getAttribute('aria-pressed'), 'true');
    assert.equal(app.context.document.body.classList.contains('no-chat'), false);
    app.get('queue-toggle').listeners.click();
    assert.equal(app.get('queue-panel').hidden, false);
    app.get('volume-toggle').listeners.click();
    assert.equal(app.get('queue-panel').hidden, true);
    assert.equal(app.get('volume-panel').hidden, false);
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.match(app.get('queue-list').children[0].textContent, /<img/);
    const source = app.get('direct-video').src;
    app.get('music-anchor').listeners.click();
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    assert.equal(app.get('direct-video').src, source);
    app.get('music-anchor').listeners.click();
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'true');
    app.get('music-volume').listeners.input({ target: { value: '60' } });
    assert.equal(app.get('direct-video').volume, 0.6);
    assert.equal(app.get('volume-value').textContent, '60%');
    app.handlers.syncQueue({ items: [item('two')], version: 2 });
    assert.equal(app.get('direct-video').volume, 0.6);
    assert.equal(app.emitted.length, 0);
});
test('overlay starts with useful defaults and replaces the welcome when data arrives', () => {
    const app = overlay();
    assert.match(app.get('desktop-clock').textContent, /^\d{2}:\d{2}$/);
    assert.ok(app.get('desktop-date').textContent.length > 0);

    app.handlers.chatMessage({ user: 'viewer', content: '¡Hola!' });
    assert.equal(app.get('chat-empty').hidden, true);
    assert.equal(app.get('chat-messages').children[0].children[1].textContent, '¡Hola!');
});
test('top area is a single desktop bar without stream status cards', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    assert.match(html, /<header id="hud" class="system-topbar"/);
    assert.match(html, /id="channel-avatar"[\s\S]*?files\.kick\.com\/images\/user\/19296849\/profile_image/);
    assert.doesNotMatch(html, /id="desktop-widget"|id="desktop-dock"|id="weather-description"|id="weather-temperature"/);
    assert.doesNotMatch(html, /api\.open-meteo|geolocation/);
    assert.doesNotMatch(html, /class="stat-chip"|class="live-badge/);
    assert.doesNotMatch(html, /AERO<span>OS|ESCRITORIO|GORILINRIX|Aero Music/);
    assert.match(html, /id="stream-title-display"[^>]*>seniordai</);
    assert.match(html, /id="music-anchor"/);
});
test('vintage corner records are decorative and cannot capture mouse interaction', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    assert.match(html, /class="corner-record corner-record-left" aria-hidden="true"/);
    assert.match(html, /class="corner-record corner-record-right" aria-hidden="true"/);
    assert.match(css, /\.corner-record \{[^}]*height: 42px;[^}]*overflow: hidden;[^}]*pointer-events: none;/);
});
test('glass theme covers overlay surfaces without changing layout or playback states', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    const css = fs.readFileSync(__dirname + '/../overlay/glass.css', 'utf8');
    assert.ok(html.indexOf('href="/glass.css"') > html.indexOf('href="/style.css"'));
    for (const selector of ['#hud.system-topbar', '#media-widget', '#bingo-widget', '.topbar-panel',
        '#alert-box', '.msg', '.chat-empty', '.tip', '#connection-status', '.corner-record-disc']) {
        assert.ok(css.includes(selector), selector);
    }
    assert.match(css, /prefers-reduced-transparency: reduce/);
    assert.match(css, /@supports not \(backdrop-filter:/);
    assert.doesNotMatch(css, /visibility:|pointer-events:|z-index:|position:|transform:/);
});
test('music unfolds only for the active visible queue and retracts on hide or disconnect', () => {
    const app = overlay();
    app.handlers.config({ channel_name: 'SeniorDai' });
    assert.equal(app.get('stream-title-display').textContent, 'seniordai');
    assert.equal(app.context.document.title, 'seniordai');
    app.handlers.playbackRole({ active: true });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'true');
    assert.equal(app.get('media-widget').getAttribute('aria-hidden'), 'false');
    assert.equal(app.get('music-anchor').getAttribute('data-playing'), 'true');
    app.handlers.toggleVideo({ showVideo: false });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    app.handlers.toggleVideo({ showVideo: true });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'true');
    app.handlers.disconnect('transport close');
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    assert.equal(app.get('music-anchor').getAttribute('data-playing'), 'false');
});
test('bingo renders drawn numbers, preserves local visibility and verifies winner presentation safely', () => {
    const app = overlay();
    const state = { round: 1, phase: 'open', drawn: [], participants: 2, winner: null, line: [], card: null };
    app.handlers.bingoState(state);
    assert.equal(app.get('bingo-widget').hidden, false);
    assert.equal(app.get('bingo-board').children.length, 5);
    assert.equal(app.get('bingo-board').children[0].children.length, 16);
    assert.equal(app.get('bingo-status').textContent, 'Inscripciones abiertas');
    app.handlers.bingoState({ ...state, phase: 'running', drawn: [1, 22, 75] });
    assert.equal(app.get('bingo-ball').textContent, 'O-75');
    assert.equal(app.get('bingo-count').textContent, '3 / 75');
    assert.equal(app.get('bingo-board').children[0].children[1].classList.contains('called'), true);
    assert.equal(app.get('bingo-board').children[4].children[15].classList.contains('latest'), true);
    app.get('bingo-toggle').listeners.click();
    app.handlers.bingoState({ ...state, phase: 'running', drawn: [1, 22, 75, 34] });
    assert.equal(app.get('bingo-widget').hidden, true);
    app.get('bingo-toggle').listeners.click();
    assert.equal(app.get('bingo-widget').hidden, false);
    const card = Array.from({ length: 25 }, (_, i) => (i % 5) * 15 + Math.floor(i / 5) + 1);
    card[12] = 0;
    const line = card.slice(0, 5);
    app.handlers.bingoState({ ...state, phase: 'won', drawn: line, winner: '<img>', card, line });
    assert.equal(app.get('bingo-status').textContent, '¡Bingo! <img>');
    assert.equal(app.get('bingo-winning-card').hidden, false);
    assert.equal(app.get('bingo-winning-card').children.length, 30);
    assert.equal(app.get('bingo-winning-card').children.filter(cell => cell.classList.contains('winning')).length, 5);
    app.handlers.bingoState({ ...state, round: 2 });
    assert.equal(app.get('bingo-winning-card').hidden, true);
    assert.equal(app.get('bingo-ball').textContent, '—');
    assert.equal(app.get('bingo-board').children[0].children[1].classList.contains('called'), false);
    app.handlers.disconnect('transport close');
    assert.equal(app.get('bingo-widget').hidden, true);
    assert.equal(app.emitted.length, 0);
});
test('bingo rejects invalid numbers and winners and restores a public view from a snapshot', () => {
    const app = overlay('');
    const state = { round: 3, phase: 'running', drawn: [5, 30], participants: 1 };
    app.handlers.bingoState(state);
    assert.equal(app.get('bingo-ball').textContent, 'I-30');
    for (const data of [{ ...state, drawn: [5, 5] }, { ...state, drawn: [76] },
        { ...state, phase: 'won', card: [], line: [], winner: 'fake' }, { ...state, participants: -1 }]) {
        app.handlers.bingoState(data);
        assert.equal(app.get('bingo-ball').textContent, 'I-30');
        assert.match(app.context.document.body.children[0].textContent, /bingo.*inválido/);
    }
    app.handlers.bingoState({ ...state, phase: 'idle', drawn: [] });
    assert.equal(app.get('bingo-widget').hidden, true);
    assert.equal(app.emitted.length, 0);
});
test('public overlay cannot play media, TTS or emit queue advances', () => {
    const app = overlay('');
    app.handlers.playbackRole({ active: false });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    app.handlers.speak({ audioBase64: 'AAAA' });
    assert.ok(!app.get('direct-video').src);
    assert.equal(app.audios.length, 0);
    assert.equal(app.emitted.length, 0);
});
test('duplicate videos advance by element ID and version; removing head switches media', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one'), item('two')], version: 2 });
    app.get('direct-video').onended(); app.get('direct-video').onended();
    assert.equal(app.emitted.length, 1);
    assert.equal(app.emitted[0][1].id, 'one'); assert.equal(app.emitted[0][1].version, 2);
    app.handlers.syncQueue({ items: [item('two', 'https://cdn.example/second.mp4')], version: 3 });
    assert.equal(app.get('direct-video').src, 'https://cdn.example/second.mp4');
    app.get('direct-video').onended();
    assert.equal(app.emitted[1][1].id, 'two');
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.equal(app.get('direct-video').src, 'https://cdn.example/second.mp4');
});
test('TTS audio is sequential and cleared on disconnect', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.speak({ audioBase64: 'AAAA' }); app.handlers.speak({ audioBase64: 'BBBB' });
    assert.equal(app.audios.length, 1); app.audios[0].onended();
    assert.equal(app.audios.length, 2);
    app.handlers.disconnect('transport close'); assert.equal(app.audios[1].paused, true);
});
test('YouTube ended/error events advance once with no custom postMessage handler', async () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [{ ...item('youtube'), videoId: 'dQw4w9WgXcQ', url: undefined }], version: 1 });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(app.players.length, 1);
    app.players[0].options.events.onStateChange({ data: 0 }); app.players[0].options.events.onError();
    assert.equal(app.emitted.length, 1);
});
test('missing channel produces guidance and does not open a socket', () => {
    const app = overlay('', ''); assert.equal(app.context.options, undefined);
});

test('index redirects with the full channel query and private token', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/index.html', 'utf8');
    const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
    let destination;
    vm.runInNewContext(script, { location: { search: '?ch=alpha', hash: '#token=private', replace(url) { destination = url; } } });
    assert.equal(destination, '/pixel.html?ch=alpha#token=private');
});

test('losing the playback role cancels media, speech and late completion events', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    app.handlers.speak({ audioBase64: 'AAAA' });
    const ended = app.get('direct-video').onended;
    app.handlers.playbackRole({ active: false });
    ended();
    assert.equal(app.get('direct-video').src, '');
    assert.equal(app.audios[0].paused, true);
    assert.equal(app.emitted.length, 0);
    assert.equal(app.timers.size, 0);
});

test('reconnection accepts the fresh server version and elects a waiting private overlay', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('old')], version: 100 });
    app.handlers.disconnect('io server disconnect');
    assert.deepEqual(app.calls, ['connect']);
    app.handlers.connect(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('fresh', 'https://cdn.example/fresh.mp4')], version: 1 });
    assert.equal(app.get('direct-video').src, 'https://cdn.example/fresh.mp4');
    app.handlers.playbackRole({ active: false });
    app.handlers.playerAvailable();
    assert.deepEqual(app.calls, ['connect', 'disconnect', 'connect']);
    const public = overlay(''); public.handlers.playerAvailable();
    assert.equal(public.calls.length, 0);
});

test('direct-video autoplay rejection preserves the head and reports guidance', async () => {
    const error = Object.assign(new Error('blocked'), { name: 'NotAllowedError' });
    const app = overlay('#token=private', '?ch=alpha', { playError: error });
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(app.emitted.length, 0);
    assert.match(app.context.document.body.children[0].textContent, /Audio bloqueado/);
});

test('YouTube API recovers for the next item after a failed network request', async () => {
    const app = overlay(); delete app.context.YT;
    app.handlers.playbackRole({ active: true });
    const video = id => ({ ...item(id), videoId: 'dQw4w9WgXcQ', url: undefined });
    app.handlers.syncQueue({ items: [video('one')], version: 1 });
    app.context.document.head.children[0].onerror();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(app.emitted.length, 1);
    assert.equal(app.context.document.head.children.length, 0);
    app.handlers.syncQueue({ items: [video('two')], version: 2 });
    assert.equal(app.context.document.head.children.length, 1);
});

test('stale media callbacks cannot advance a new head and pending ACKs retry safely', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    const ended = app.get('direct-video').onended;
    ended();
    const retry = [...app.timers.values()].at(-1);
    retry();
    assert.equal(app.emitted.length, 2);
    assert.equal(app.emitted[1][1].id, 'one');
    assert.equal(app.emitted[1][1].version, 1);
    app.handlers.syncQueue({ items: [item('two')], version: 2 });
    ended(); retry();
    assert.equal(app.emitted.length, 2);
    app.handlers.syncQueue({ items: [], version: 3 });
    assert.equal(app.get('direct-video').src, '');
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
});
