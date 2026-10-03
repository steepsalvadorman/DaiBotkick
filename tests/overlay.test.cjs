const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function overlay(hash = '#token=private', search = '?ch=alpha') {
    class Element {
        constructor() { this.children = []; this.style = {}; this.textContent = ''; this.attributes = {}; this.classList = { add() {}, remove() {} }; }
        append(...nodes) { for (const node of nodes) { node.parent = this; this.children.push(node); } }
        replaceChildren(...nodes) { this.children = []; this.append(...nodes); }
        get firstChild() { return this.children[0]; }
        remove() { this.parent.children.splice(this.parent.children.indexOf(this), 1); }
        setAttribute(name, value) { this.attributes[name] = value; }
        getAttribute(name) { return this.attributes[name]; }
        removeAttribute(name) { delete this.attributes[name]; if (name === 'src') this.src = ''; }
        pause() { this.paused = true; }
        load() {}
        play() { this.paused = false; return Promise.resolve(); }
        set innerHTML(_) { throw new Error('Unsafe HTML insertion'); }
    }
    const elements = new Map();
    const get = id => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); };
    const handlers = {}, emitted = [], audios = [], players = [];
    let timerId = 0;
    const timers = new Map();
    const socket = { connected: true, on(name, callback) { handlers[name] = callback; }, emit(...args) { emitted.push(args); }, disconnect() {}, connect() {} };
    const context = {
        document: { body: new Element(), head: new Element(), getElementById: get, createElement: () => new Element(), createTextNode: text => ({ textContent: text }), querySelectorAll: () => [] },
        location: { hash, search, origin: 'https://bot.example' },
        innerWidth: 1920, innerHeight: 1080, URLSearchParams, Date, Promise,
        io: options => { context.options = options; return socket; }, addEventListener() {},
        setTimeout(fn) { timers.set(++timerId, fn); return timerId; }, clearTimeout(id) { timers.delete(id); }, setInterval() {},
        Audio: class extends Element { constructor(src) { super(); this.src = src; audios.push(this); } },
        YT: { PlayerState: { PLAYING: 1, ENDED: 0 }, Player: class {
            constructor(target, options) { this.options = options; players.push(this); }
            destroy() { this.destroyed = true; } playVideo() {} setVolume() {}
        } },
    };
    context.window = context;
    vm.runInNewContext(fs.readFileSync(__dirname + '/../overlay/app.js', 'utf8'), context);
    return { handlers, emitted, audios, players, get, context, timers };
}
const item = (id, url = 'https://cdn.example/video.mp4') => ({ id, title: '<img src=x onerror=alert(1)>', user: 'viewer', url });

test('external chat, config and titles are rendered as literal text', () => {
    const app = overlay();
    app.handlers.config({ channel_name: '<script>', kick_url: 'kick.com/a' });
    app.handlers.chatMessage({ user: '<img>', content: '<script>alert(1)</script>' });
    assert.equal(app.get('chat-messages').children[0].children[1].textContent, '<script>alert(1)</script>');
    app.handlers.playbackRole({ active: true });
    app.handlers.syncQueue({ items: [item('one')], version: 1 });
    assert.match(app.get('video-title').children[0].textContent, /<img/);
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
