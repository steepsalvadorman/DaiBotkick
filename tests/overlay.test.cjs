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
        replaceWith(node) { const index = this.parent.children.indexOf(this); this.parent.children.splice(index, 1, node); node.parent = this.parent; }
        get firstChild() { return this.children[0]; }
        remove() { this.removed = true; this.parent.children.splice(this.parent.children.indexOf(this), 1); }
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
    const handlers = {}, emitted = [], audios = [], players = [], calls = [], windowListeners = {};
    let timerId = 0, intervalCallback;
    const timers = new Map();
    const socket = { connected: true, on(name, callback) { handlers[name] = callback; }, emit(...args) { emitted.push(args); }, disconnect() { calls.push('disconnect'); }, connect() { calls.push('connect'); } };
    const context = {
        document: { body: new Element(), head: new Element(), getElementById: id => get(id), createElement: () => new Element(), createTextNode: text => ({ textContent: text }), querySelectorAll: selector => selector === '.tip' ? options.tips || [] : [] },
        location: { hash, search, origin: 'https://bot.example' },
        localStorage: options.storage, innerWidth: options.width ?? 1920, innerHeight: options.height ?? 1080, URLSearchParams, Date, Promise, Intl,
        CSS: { supports: (property, value) => property === 'zoom' && value === '1' && options.nativeScale !== false },
        io: options => { context.options = options; return socket; }, addEventListener(name, callback) { windowListeners[name] = callback; },
        setTimeout(fn) { timers.set(++timerId, fn); return timerId; }, clearTimeout(id) { timers.delete(id); }, setInterval(fn, delay) { (context.intervals ||= []).push({ fn, delay }); intervalCallback = fn; },
        Audio: class extends Element { constructor(src) { super(); this.src = src; audios.push(this); } },
        YT: { PlayerState: { PLAYING: 1, PAUSED: 2, ENDED: 0 }, Player: class {
            constructor(target, options) { this.options = options; players.push(this); }
            destroy() { this.destroyed = true; } getCurrentTime() { return this.now ?? 0; } playVideo() { this.paused = false; } pauseVideo() { this.paused = true; } seekTo(time) { this.time = time; } setVolume(value) { this.volume = value; }
        } },
    };
    context.window = context;
    vm.runInNewContext(fs.readFileSync(__dirname + '/../overlay/app.js', 'utf8'), context);
    return { handlers, emitted, audios, players, get, context, timers, calls, socket, windowListeners, intervalCallback: () => intervalCallback };
}
const item = (id, url = 'https://cdn.example/video.mp4') => ({ id, title: '<img src=x onerror=alert(1)>', user: 'viewer', url });

test('overlay renders at native output scale and fits every viewport without cropping', () => {
    const sizes = [[1280, 720], [1920, 1080], [2560, 1440], [3840, 2160], [7680, 4320],
        [3440, 1440], [1920, 1200], [1024, 768], [390, 844], [844, 390], [1365, 767]];
    for (const nativeScale of [true, false]) {
        for (const [width, height] of sizes) {
            const app = overlay('', '', { width, height, nativeScale });
            const style = app.context.document.body.style;
            const scale = Math.min(width / 1920, height / 1080);
            assert.equal(style.zoom, nativeScale ? String(scale) : '1');
            assert.equal(style.transform, nativeScale ? 'none' : `scale(${scale})`);
            const left = parseFloat(style.marginLeft) * (nativeScale ? scale : 1);
            const top = parseFloat(style.marginTop) * (nativeScale ? scale : 1);
            assert.ok(Math.abs(left * 2 + 1920 * scale - width) < 0.000001);
            assert.ok(Math.abs(top * 2 + 1080 * scale - height) < 0.000001);
            assert.ok(left >= 0 && top >= 0);
        }
    }
});
test('resizing preserves the composition and ignores temporarily empty viewports', () => {
    for (const nativeScale of [true, false]) {
        const app = overlay('', '', { nativeScale });
        app.context.innerWidth = 3840;
        app.context.innerHeight = 2160;
        app.windowListeners.resize();
        const style = app.context.document.body.style;
        assert.equal(style.zoom, nativeScale ? '2' : '1');
        assert.equal(style.transform, nativeScale ? 'none' : 'scale(2)');
        for (const [width, height] of [[0, 2160], [3840, 0], [0, 0]]) {
            app.context.innerWidth = width;
            app.context.innerHeight = height;
            app.windowListeners.resize();
            assert.equal(style.zoom, nativeScale ? '2' : '1');
            assert.equal(style.transform, nativeScale ? 'none' : 'scale(2)');
        }
        app.context.innerWidth = 1920;
        app.context.innerHeight = 1200;
        app.windowListeners.resize();
        assert.equal(style.marginTop, '60px');
        assert.equal(style.marginLeft, '0px');
    }
});
test('mobile viewport uses device pixels without a fixed desktop layout viewport', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    assert.match(html, /name="viewport" content="width=device-width, initial-scale=1"/);
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    assert.match(css, /body \{[^}]*position: relative;/);
});

test('music transport pauses, resumes, restarts and skips direct media safely', async () => {
    const app = overlay();
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    const video = app.get('direct-video');
    video.onplaying();
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'playing');
    assert.equal(app.get('media-widget').getAttribute('data-state'), 'playing');
    app.get('music-pause').listeners.click();
    assert.equal(video.paused, true);
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'paused');
    assert.equal(app.get('media-widget').getAttribute('data-state'), 'paused');
    video.currentTime = 40;
    app.get('music-restart').listeners.click();
    assert.equal(video.currentTime, 0);
    assert.equal(video.paused, true);
    app.get('music-pause').listeners.click();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(video.paused, false);
    video.onplaying();
    video.currentTime = 12;
    app.get('music-stop').listeners.click();
    assert.equal(video.paused, true);
    assert.equal(video.currentTime, 0);
    assert.equal(app.emitted.length, 0);
    video.onplaying();
    app.get('music-next').listeners.click();
    app.get('music-next').listeners.click();
    assert.equal(app.emitted.length, 1);
    assert.equal(app.emitted[0][1].id, 'one');
    app.handlers.disconnect('transport close');
    assert.equal(app.get('music-pause').disabled, true);
    assert.equal(app.get('music-transport').getAttribute('data-state'), 'idle');
    assert.equal(app.get('media-widget').getAttribute('data-state'), 'idle');
});
test('YouTube transport uses player methods and public transport remains disabled', async () => {
    const app = overlay();
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [{ ...item('yt'), videoId: 'dQw4w9WgXcQ' }], version: 1 });
    await new Promise(resolve => setImmediate(resolve));
    const player = app.players[0];
    player.options.events.onReady({ target: player });
    assert.equal(player.volume, 60);
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
test('chat preserves Unicode emoji and renders Kick emotes safely with explicit image failure fallback', () => {
    const app = overlay();
    const emoji = 'Hola 😀 💙 👨‍👩‍👧‍👦';
    app.handlers.chatMessage({ user: 'viewer', content: emoji });
    assert.equal(app.get('chat-messages').children[0].children[1].textContent, emoji);
    app.handlers.chatMessage({ user: 'viewer', content: 'Hola [emote:37226:KEKW] 😀 [emote:4148074:<img>] fin' });
    const text = app.get('chat-messages').children[1].children[1];
    assert.equal(text.children[0].textContent, 'Hola ');
    assert.equal(text.children[1].src, 'https://files.kick.com/emotes/37226/fullsize');
    assert.equal(text.children[1].alt, 'KEKW');
    assert.equal(text.children[2].textContent, ' 😀 ');
    assert.equal(text.children[3].alt, '<img>');
    assert.equal(text.children[4].textContent, ' fin');
    text.children[1].listeners.error();
    assert.equal(text.children[1].textContent, ':KEKW:');
    assert.match(app.context.document.body.children[0].textContent, /No se pudo cargar el emote KEKW/);
    app.handlers.chatMessage({ user: 'viewer', content: '[emote:https://evil.example:x] <script>' });
    assert.equal(app.get('chat-messages').children[2].children[1].textContent, '[emote:https://evil.example:x] <script>');
});
test('topbar controls toggle local panels, chat and player without changing queue permissions', () => {
    const app = overlay('#token=private', '?ch=alpha');
    assert.equal(app.get('chat-toggle').getAttribute('aria-pressed'), 'false');
    assert.equal(app.context.document.body.classList.contains('no-chat'), true);
    app.get('chat-toggle').listeners.click();
    assert.equal(app.get('chat-toggle').getAttribute('aria-pressed'), 'true');
    assert.equal(app.context.document.body.classList.contains('no-chat'), false);
    const shown = overlay('#token=private', '?ch=alpha&chat=1');
    assert.equal(shown.get('chat-toggle').getAttribute('aria-pressed'), 'true');
    assert.equal(shown.context.document.body.classList.contains('no-chat'), false);
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
test('voice volume is controlled separately from music volume', () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.get('tts-volume').listeners.input({ target: { value: '40' } });
    assert.equal(app.get('tts-volume-value').textContent, '40%');
    app.handlers.speak({ audioBase64: 'AAAA' });
    assert.equal(app.audios.at(-1).volume, 0.4);
    app.get('music-volume').listeners.input({ target: { value: '20' } });
    assert.equal(app.audios.at(-1).volume, 0.4);
    app.get('tts-volume').listeners.input({ target: { value: '100' } });
    assert.equal(app.audios.at(-1).volume, 1);
    app.get('tts-volume').listeners.input({ target: { value: '150' } });
    assert.equal(app.audios.at(-1).volume, 1);
    for (const page of ['pixel.html', 'vsaturn.html']) {
        const html = fs.readFileSync(__dirname + `/../overlay/${page}`, 'utf8');
        assert.match(html, /id="tts-volume" type="range" min="0" max="100" value="100"/, page);
        assert.match(html, /id="tts-volume-value" for="tts-volume">100%</, page);
    }
});
test('overlay starts with useful defaults and an empty chat until messages arrive', () => {
    const app = overlay();
    assert.match(app.get('desktop-clock').textContent, /^\d{2}:\d{2}$/);
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    assert.match(html, /id="topbar-requester" class="topbar-requester" hidden>/);
    assert.doesNotMatch(html, /desktop-date|chat-empty|El chat está listo/);
    assert.match(html, /id="music-volume" type="range" min="0" max="100" value="60"/);
    assert.match(html, /id="volume-value" for="music-volume">60%</);
    assert.match(fs.readFileSync(__dirname + '/../overlay/app.js', 'utf8'), /musicVolume = 60, ttsVolume = 100;/);

    app.handlers.chatMessage({ user: 'viewer', content: '¡Hola!' });
    assert.equal(app.get('chat-messages').children[0].children[1].textContent, '¡Hola!');
});
test('chat keeps the latest four messages', () => {
    const app = overlay();
    const body = app.get('chat-messages');
    for (let i = 0; i < 10; i++) app.handlers.chatMessage({ user: `viewer${i}`, content: `Mensaje ${i}` });
    assert.equal(body.children.length, 4);
    assert.equal(body.children[0].children[1].textContent, 'Mensaje 6');
    assert.equal(body.children[3].children[1].textContent, 'Mensaje 9');
});
test('top area is a single desktop bar without stream status cards', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    assert.match(html, /<header id="hud" class="system-topbar"/);
    assert.match(html, /id="channel-avatar"\s+src="\/daibot\.svg"\s+alt="DaiBot, ninja veloz electrónico y programador"/);
    assert.doesNotMatch(html, /files\.kick\.com\/images\/user\/19296849\/profile_image/);
    assert.doesNotMatch(html, /id="desktop-widget"|id="desktop-dock"|id="weather-description"|id="weather-temperature"/);
    assert.doesNotMatch(html, /api\.open-meteo|geolocation/);
    assert.doesNotMatch(html, /class="stat-chip"|class="live-badge/);
    assert.doesNotMatch(html, /AERO<span>OS|ESCRITORIO|DAIBOT|Aero Music/);
    assert.match(html, /id="stream-title-display"[^>]*>seniordai</);
    assert.match(html, /id="music-anchor"/);
});
test('original kinetic artwork stays decorative and the game canvas stays transparent', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    assert.match(html, /class="kinetic-corner kinetic-corner-left" aria-hidden="true"/);
    assert.match(html, /class="kinetic-corner kinetic-corner-right" aria-hidden="true"/);
    assert.match(css, /\.kinetic-corner \{[^}]*height: 68px;[^}]*overflow: hidden;[^}]*pointer-events: none;/);
    assert.match(css, /html, body \{[^}]*background: transparent;/);
    assert.match(css, /url\('\/kinetic-burst\.svg'\)/);
    assert.ok(fs.existsSync(__dirname + '/../overlay/kinetic-burst.svg'));
    const hand = fs.readFileSync(__dirname + '/../overlay/daibot.svg', 'utf8');
    assert.doesNotMatch(hand, /<image|href=|<script|on\w+=/);
    assert.doesNotMatch(html, /widget-heading|heading-sticker|heading-mark|>Chat en vivo</);
    assert.match(html, /class="alert-art channel-avatar-sync" src="\/daibot\.svg" alt="" aria-hidden="true"/);
});
test('visual system pairs comic headings with readable body type and preserves scale and motion', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/pixel.html', 'utf8');
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    assert.match(html, /family=Manrope:wght@400;500;600;700&display=swap/);
    assert.match(html, /family=Bangers&family=Manrope/);
    assert.match(css, /--display: 'Bangers', 'Impact', 'Haettenschweiler', sans-serif;/);
    for (const selector of ['#hud .stream-title', '.aero-window-title', '.alert-header', '.alert-body', '.tip-label']) {
        const start = css.indexOf(`${selector} {`);
        assert.notEqual(start, -1);
        assert.match(css.slice(start, css.indexOf('}', start)), /font-family: var\(--display\);/);
    }
    assert.match(css, /body \{[^}]*font-family: var\(--font\);/);
    assert.doesNotMatch(html + css, /DM\+Sans|DM Sans|Space Mono|Inter\b|Roboto|Helvetica|Arial/);
    assert.doesNotMatch(css, /italic|font-weight: (800|900)/);
    assert.doesNotMatch(css, /gradient\(/);
    for (const [, size] of css.matchAll(/font-size: ([^;]+);/g)) assert.ok(['12px', '14px', '16px', '18px', '20px', '24px', '30px', '36px', '48px'].includes(size), size);
    for (const [, radius] of css.matchAll(/border-radius: ([^;]+);/g)) assert.ok(['0', '2px', '4px', '6px', '8px', '12px', '16px', '24px', '9999px'].includes(radius), radius);
    assert.doesNotMatch(css, /border-(left|right|top|bottom):/);
    for (const [, rule] of css.matchAll(/(?:transition|animation): ([^;]+);/g)) {
        if (rule === 'none !important') continue;
        assert.doesNotMatch(rule, /\b(ease|linear|ease-in|ease-out|ease-in-out)\b/, rule);
        assert.match(rule, /var\(--(mass|spring|swing)\)/, rule);
    }
    assert.match(css, /--mass: cubic-bezier\(0\.32, 0\.72, 0, 1\);/);
});
test('nickname reserves room for comic glyph overhang while retaining long-name truncation', () => {
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    const start = css.indexOf('#hud .stream-title {');
    const title = css.slice(start, css.indexOf('}', start));
    assert.match(title, /padding-inline: 4px 8px;/);
    assert.match(title, /max-width: 240px;/);
    assert.match(title, /overflow: hidden;/);
    assert.match(title, /text-overflow: ellipsis;/);
});
test('reduced motion leaves one readable tip and disables decorative movement', () => {
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    const reduced = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
    assert.match(reduced, /animation: none !important; transition: none !important;/);
    assert.match(reduced, /\.tip \{ display: none; \}/);
    assert.match(reduced, /\.tip:first-child \{ display: flex; opacity: 1; \}/);
});
test('lite mode is the default and stops continuous animations and rotates tips without CSS keyframes', () => {
    const tips = [0, 1, 2].map(() => { const classes = new Set(); return { classes, classList: { toggle(name, force) { if (force) classes.add(name); else classes.delete(name); } } }; });
    const lite = overlay('#token=private', '?ch=alpha', { tips });
    assert.ok(lite.context.document.body.classList.contains('lite'));
    assert.deepEqual(tips.map(tip => tip.classes.has('current')), [true, false, false]);
    const rotate = lite.context.intervals.find(i => i.delay === 9000).fn;
    rotate(); assert.deepEqual(tips.map(tip => tip.classes.has('current')), [false, true, false]);
    rotate(); rotate(); assert.deepEqual(tips.map(tip => tip.classes.has('current')), [true, false, false]);

    const normal = overlay('#token=private', '?ch=alpha&lite=0', { tips });
    assert.ok(!normal.context.document.body.classList.contains('lite'));

    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    const block = css.slice(css.indexOf('body.lite .kinetic-corner::before'), css.indexOf('@media (prefers-reduced-motion: reduce)'));
    for (const selector of ['.kinetic-corner::before', '.topbar-center::after', '#alert-box::before', '.brand-live::before', '.vinyl-record', '.music-bars i', '.title-inner.marquee', '.tip::after']) {
        assert.ok(block.includes(`body.lite ${selector}`), selector);
    }
    assert.match(block, /\{ animation: none !important; \}/);
    assert.match(block, /body\.lite \.tip\.current \{ display: flex; \}/);
    assert.match(css, /#alert-box:not\(\.show\)::before[^{]*\{ animation-play-state: paused; \}/);
});
test('clock skips DOM writes when nothing changed', () => {
    const app = overlay();
    const clock = app.get('desktop-clock');
    let writes = 0, value = clock.textContent;
    Object.defineProperty(clock, 'textContent', { get: () => value, set: text => { writes++; value = text; } });
    app.context.intervals.find(i => i.delay === 1000).fn();
    assert.equal(writes, 0);
});
test('winner presentation replaces the drawn board to leave the chat unobstructed', () => {
    const css = fs.readFileSync(__dirname + '/../overlay/style.css', 'utf8');
    assert.match(css, /#bingo-widget\[data-phase="won"\] #bingo-board,\s*#bingo-widget\[data-phase="won"\] #bingo-history \{ display: none; \}/);
    assert.match(css, /\.chat-body \{[^}]*justify-content: flex-end;/);
});
test('music unfolds only for the active visible queue and retracts on hide or disconnect', () => {
    const app = overlay();
    app.handlers.config({ channel_name: 'SeniorDai' });
    assert.equal(app.get('stream-title-display').textContent, 'SeniorDai');
    assert.equal(app.context.document.title, 'SeniorDai');
    app.handlers.config({ channel_name: 'Frank_Arr', kick_url: 'kick.com/frank-arr' });
    assert.equal(app.get('stream-title-display').textContent, 'Frank_Arr');
    assert.equal(app.get('channel-avatar').src, undefined);
    app.handlers.config({ channel_name: 'Frank_Arr', avatar_url: 'https://files.kick.com/images/user/1/profile_image/frank.webp' });
    assert.equal(app.get('channel-avatar').src, 'https://files.kick.com/images/user/1/profile_image/frank.webp');
    assert.equal(app.get('channel-avatar').alt, 'Avatar de Frank_Arr');
    app.handlers.config({ channel_name: 'Frank_Arr', avatar_url: 'javascript:alert(1)' });
    assert.equal(app.get('channel-avatar').src, 'https://files.kick.com/images/user/1/profile_image/frank.webp');
    app.handlers.playbackRole({ active: true });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'true');
    assert.equal(app.get('media-widget').getAttribute('aria-hidden'), 'false');
    assert.equal(app.get('music-anchor').getAttribute('data-playing'), 'true');
    assert.equal(app.get('topbar-requester').hidden, false);
    assert.equal(app.get('video-requester').textContent, 'viewer');
    app.handlers.toggleVideo({ showVideo: false });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    app.handlers.toggleVideo({ showVideo: true });
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'true');
    app.handlers.disconnect('transport close');
    assert.equal(app.get('media-widget').getAttribute('data-shown'), 'false');
    assert.equal(app.get('music-anchor').getAttribute('data-playing'), 'false');
    assert.equal(app.get('topbar-requester').hidden, true);
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
    app.handlers.speak({ audioBase64: '' }); app.handlers.speak(null);
    assert.equal(app.audios.length, 1); assert.equal(app.audios[0].src, 'data:audio/mp3;base64,AAAA');
    app.audios[0].onended();
    assert.equal(app.audios.length, 2); assert.equal(app.audios[1].src, 'data:audio/mp3;base64,BBBB');
    app.handlers.disconnect('transport close'); assert.equal(app.audios[1].paused, true);
});
test('legacy TikTok queue items are skipped without building an embed', async () => {
    const app = overlay(); app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [{ ...item('tt', null), tiktokId: '7440114099565972791' }], version: 1 });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(app.get('youtube-player').children.length, 0);
    assert.equal(JSON.stringify(app.emitted), JSON.stringify([['advanceQueue', { id: 'tt', version: 1 }]]));
});
test('chat messages with emoji do not render an emoji meter', () => {
    const app = overlay();
    app.handlers.chatMessage({ user: 'viewer', content: '😀😀 [emote:37226:KEKW]' });
    assert.ok(!app.context.intervals.some(i => i.delay === 5000));
    for (const page of ['pixel.html', 'vsaturn.html']) {
        assert.doesNotMatch(fs.readFileSync(__dirname + `/../overlay/${page}`, 'utf8'), /emoji-meter/, page);
    }
});
test('a reloaded OBS source resumes the same queue head where it stopped and forgets it when it ends', async () => {
    const data = new Map();
    const storage = { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value), removeItem: key => data.delete(key) };
    const tick = app => app.context.intervals.find(i => i.delay === 2000).fn();
    const queueOf = extra => ({ items: [{ ...item('head', null), ...extra }, item('next')], version: 1 });
    const start = async extra => {
        const app = overlay('#token=private', '?ch=alpha', { storage });
        app.handlers.playbackRole({ active: true }); app.handlers.syncQueue(queueOf(extra));
        await new Promise(resolve => setImmediate(resolve));
        return app;
    };

    let app = await start({ videoId: 'dQw4w9WgXcQ' });
    app.players[0].options.events.onReady({ target: app.players[0] });
    app.players[0].now = 95.5; tick(app);
    assert.equal(data.get('daibot:resume:alpha'), JSON.stringify({ id: 'head', time: 95.5 }));
    app = await start({ videoId: 'dQw4w9WgXcQ' });
    app.players[0].options.events.onReady({ target: app.players[0] });
    assert.equal(app.players[0].time, 93.5);
    app.players[0].options.events.onStateChange({ data: 0 });
    assert.equal(data.has('daibot:resume:alpha'), false);

    data.set('daibot:resume:alpha', JSON.stringify({ id: 'head', time: 12 }));
    app = await start({ url: 'https://cdn.example/video.mp4' });
    app.get('direct-video').onloadedmetadata();
    assert.equal(app.get('direct-video').currentTime, 10);

    data.set('daibot:resume:alpha', JSON.stringify({ id: 'old', time: 300 }));
    app = await start({ url: 'https://cdn.example/video.mp4' });
    assert.equal(app.get('direct-video').onloadedmetadata, undefined);
    assert.equal(data.has('daibot:resume:alpha'), false);
});
test('YouTube ended/error events advance once', async () => {
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

test('every overlay theme page provides the elements app.js drives', () => {
    const app = fs.readFileSync(__dirname + '/../overlay/app.js', 'utf8');
    const ids = new Set([...app.matchAll(/el\('([\w-]+)'\)/g)].map(match => match[1]));
    for (const name of ['queue', 'volume']) { ids.add(`${name}-panel`); ids.add(`${name}-toggle`); }
    for (const page of ['pixel.html', 'vsaturn.html']) {
        const html = fs.readFileSync(__dirname + '/../overlay/' + page, 'utf8');
        for (const id of ids) assert.match(html, new RegExp(`id="${id}"`), `${page} no tiene #${id}`);
        assert.match(html, /name="viewport" content="width=device-width, initial-scale=1"/);
        assert.match(html, /class="[^"]*\bkick-channel-url\b/);
        assert.match(html, /class="[^"]*\bchannel-avatar-sync\b/);
        assert.equal(html.match(/class="tip"/g)?.length, 3);
        assert.match(html, /<script src="\/app\.js"><\/script>/);
    }
    const css = fs.readFileSync(__dirname + '/../overlay/vsaturn.css', 'utf8');
    assert.match(css, /body \{[^}]*position: relative;/);
    assert.match(css, /body\.lite \.tip\.current \{ display: flex; \}/);
    assert.doesNotMatch(css, /^\s*backdrop-filter\s*:/m);
});
