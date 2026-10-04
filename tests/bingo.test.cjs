const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

const token = 'a'.repeat(32);
const card = Array.from({ length: 25 }, (_, i) => i === 12 ? 0 : i % 5 * 15 + Math.floor(i / 5) + 1);
const state = (changes = {}) => ({ user: '<img src=x>', round: 1, phase: 'running', card, drawn: [1], winner: null, ready: false, ...changes });

test('bingo page uses the channel mascot and the simple Bingo name', () => {
    const html = fs.readFileSync(__dirname + '/../overlay/bingo.html', 'utf8');
    assert.match(html, /<title>Bingo<\/title>/);
    assert.match(html, /<h1>Bingo<\/h1>/);
    assert.match(html, /src="gorilinrix\.svg" alt="GorilinRix, mascota del canal"/);
    assert.ok(fs.existsSync(__dirname + '/../overlay/gorilinrix.svg'));
    assert.ok(!html.includes('Bingo Aero'));
});

async function page(responses, hash = `#ch=alpha&token=${token}`, storage = new Map()) {
    class Element {
        constructor() {
            this.children = []; this.listeners = {}; this.attributes = {}; this.textContent = '';
            const classes = new Set();
            this.classList = { toggle(name, enabled) { if (enabled) classes.add(name); else classes.delete(name); }, contains(name) { return classes.has(name); } };
        }
        setAttribute(name, value) { this.attributes[name] = value; }
        addEventListener(name, fn) { this.listeners[name] = fn; }
        replaceChildren(...children) { this.children = children; }
        set innerHTML(_) { throw new Error('Unsafe HTML'); }
    }
    const elements = new Map(), requests = [], timers = [];
    const get = id => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); };
    vm.runInNewContext(fs.readFileSync(__dirname + '/../overlay/bingo.js', 'utf8'), {
        document: { getElementById: get, createElement: () => new Element() },
        location: { hash }, URLSearchParams,
        localStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) },
        fetch: async (url, options) => {
            requests.push({ url, options });
            const response = responses.shift();
            if (response instanceof Error) throw response;
            return { ok: response.status === 200, status: response.status, json: async () => response.body };
        },
        setTimeout: callback => timers.push(callback),
    });
    const settle = async () => { await new Promise(resolve => setImmediate(resolve)); };
    await settle();
    return { get, requests, timers, storage, settle };
}

test('personal card displays 25 safe accessible cells, local manual marks and real draws', async () => {
    const app = await page([{ status: 200, body: state() }, { status: 200, body: state({ drawn: [1, 16], ready: true }) }]);
    const cells = app.get('card').children;
    assert.equal(cells.length, 25);
    assert.equal(app.get('title').textContent, 'Cartón de <img src=x>');
    assert.equal(cells[12].textContent, 'LIBRE');
    assert.equal(cells[12].disabled, true);
    assert.equal(cells[12].attributes['aria-pressed'], 'true');
    assert.equal(cells[0].classList.contains('drawn'), true);
    assert.equal(cells[0].attributes['aria-pressed'], 'false');
    cells[0].listeners.click();
    assert.equal(cells[0].attributes['aria-pressed'], 'true');
    const saved = app.storage.get(`bingo-marks:alpha:${token}`);
    assert.deepEqual(JSON.parse(saved), [0]);
    await app.timers.shift()();
    assert.equal(app.get('card').children[0], cells[0]);
    assert.equal(cells[1].classList.contains('drawn'), true);
    assert.match(app.get('claim').textContent, /!bingo/);
    assert.equal(app.get('claim').classList.contains('ready'), true);
    cells[0].listeners.click();
    assert.equal(cells[0].attributes['aria-pressed'], 'false');
    assert.equal(app.requests[0].url, '/api/bingo/alpha');
    assert.equal(app.requests[0].options.headers['x-bingo-token'], token);
    assert.equal(app.requests[0].options.cache, 'no-store');
});

test('saved marks restore without altering server card and expired links stop polling', async () => {
    const storage = new Map([[`bingo-marks:alpha:${token}`, '[0,3]']]);
    const app = await page([{ status: 200, body: state() }, { status: 404 }], undefined, storage);
    assert.equal(app.get('card').children[3].attributes['aria-pressed'], 'true');
    await app.timers.shift()();
    assert.match(app.get('error').textContent, /expiró/);
    assert.equal(app.timers.length, 0);
    assert.ok(app.get('card').children.every(cell => cell.disabled));
});

test('missing links do not fetch, network errors retry and winner text is safe', async () => {
    const missing = await page([], '');
    assert.equal(missing.requests.length, 0);
    assert.match(missing.get('error').textContent, /!carton/);
    const app = await page([new Error('Sin red'), { status: 200, body: state({ phase: 'won', winner: '<script>' }) }]);
    assert.match(app.get('status').textContent, /interrumpida/);
    await app.timers.shift()();
    assert.equal(app.get('status').textContent, 'Partida terminada · ganador: <script>');
    assert.equal(app.get('error').textContent, '');
    assert.equal(app.timers.length, 0);
    assert.ok(app.get('card').children.every(cell => cell.disabled));
});

test('private cards require Kick login and hide previous data when a session expires', async () => {
    const anonymous = await page([{ status: 401 }]);
    assert.equal(anonymous.get('card').children.length, 0);
    assert.equal(anonymous.get('login').hidden, false);
    assert.equal(anonymous.get('login').href, `/auth/bingo?ch=alpha&token=${token}`);
    assert.equal(anonymous.timers.length, 0);
    assert.match(anonymous.get('error').textContent, /Inicia sesión/);
    assert.equal(anonymous.requests[0].options.credentials, 'same-origin');
    const wrong = await page([{ status: 403 }]);
    assert.match(wrong.get('error').textContent, /otra cuenta/);
    assert.equal(wrong.get('card').children.length, 0);
    const expired = await page([{ status: 200, body: state() }, { status: 401 }]);
    assert.equal(expired.get('card').children.length, 25);
    await expired.timers.shift()();
    assert.equal(expired.get('card').children.length, 0);
    assert.equal(expired.get('title').textContent, 'Tu cartón personal');
    assert.equal(expired.timers.length, 0);
});
