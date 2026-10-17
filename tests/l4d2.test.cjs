const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

const fixture = () => ({
    steam_enabled: true,
    snapshot: { updated: Date.now() / 1000, server: {
        name: 'Servidor L4D2', map: 'c1m1_hotel', online: 1, capacity: 4,
        players: [{ name: '<img src=x>', score: 42, seconds: 120 }],
    } },
    profiles: [{ steamid: '76561197960265729', updated: Date.now() / 1000,
        profile: { name: '<script>Alice</script>', minutes: 120,
            stats: [{ name: 'kills', value: 123 }], achievements: { unlocked: 1, total: 2 } } }],
    links: [{ name: '<img src=x>', steamid: '76561197960265729', verified: false }],
});

async function app({ search = '?ch=alpha', hash = '', data = fixture(), fail, me = null } = {}) {
    class Element {
        constructor(tag = 'div') {
            this.tag = tag; this.children = []; this.attributes = {}; this.listeners = {};
            this.value = ''; this.hidden = false; this.disabled = false;
            const classes = new Set();
            this.classList = { toggle(name, on) { if (on) classes.add(name); else classes.delete(name); }, contains(name) { return classes.has(name); } };
        }
        set textContent(value) { this.text = String(value); this.children = []; }
        get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); }
        set innerHTML(_) { throw Error('Unsafe HTML'); }
        append(...items) { this.children.push(...items); }
        replaceChildren(...items) { this.text = ''; this.children = items; }
        addEventListener(name, handler) { this.listeners[name] = handler; }
        setAttribute(key, value) { this.attributes[key] = value; }
        removeAttribute(key) { delete this.attributes[key]; }
        querySelectorAll() { return []; }
    }
    const elements = new Map(), requests = [], timers = [], history = [];
    const get = id => {
        if (!elements.has(id)) elements.set(id, new Element());
        return elements.get(id);
    };
    const context = {
        document: { documentElement: new Element(), getElementById: get, createElement: tag => new Element(tag) },
        location: { search, hash, pathname: '/l4d2.html' },
        history: { replaceState(...args) { history.push(args); } },
        URLSearchParams, URL, Intl, Date, Set, Error,
        setTimeout(fn) { timers.push(fn); },
        async fetch(path, options) {
            requests.push({ path, options });
            if (fail?.(path, options)) return { ok: false, status: 503, text: async () => 'Servicio no disponible' };
            const response = path.endsWith('/account') ? { steamid: me }
                : path.endsWith('/admin') ? { server: '', allowed_servers: ['8.8.8.8:27015'] } : data;
            return { ok: true, status: options.method === 'POST' || options.method === 'DELETE' ? 204 : 200, json: async () => response };
        },
    };
    vm.runInNewContext(fs.readFileSync(__dirname + '/../overlay/l4d2.js', 'utf8'), context);
    await new Promise(resolve => setImmediate(resolve));
    return { context, get, requests, timers, history, data };
}

test('renders A2S and manually linked historical stats as literal text, not HTML', async () => {
    const view = await app();
    assert.match(view.get('players').textContent, /<img src=x>/);
    assert.match(view.get('players').textContent, /<script>Alice<\/script>/);
    assert.match(view.get('players').textContent, /Asociación manual · no verificada/);
    assert.match(view.get('players').textContent, /2 h en L4D2/);
    assert.match(view.get('profiles').textContent, /kills123/);
    assert.equal(view.get('steam-login').href, '/auth/steam?ch=alpha');
    assert.ok(view.requests.every(request => !request.options.headers?.Authorization));
});

test('duplicate A2S names are not automatically assigned a Steam identity', async () => {
    const data = fixture();
    data.snapshot.server.players.push({ ...data.snapshot.server.players[0] });
    const view = await app({ data });
    assert.match(view.get('players').textContent, /Nombre duplicado · identidad ambigua/);
    assert.doesNotMatch(view.get('players').textContent, /Asociación manual/);
    assert.equal(view.get('player-name').children.length, 0);
});

test('stale server data and failed refreshes display explicit warnings', async () => {
    const data = fixture();
    data.snapshot.updated -= 100;
    let broken = false;
    const view = await app({ data, fail: () => broken });
    assert.match(view.get('status').textContent, /atrasada/);
    assert.equal(view.get('status').attributes['data-error'], 'true');
    broken = true;
    await view.timers[0]();
    assert.match(view.get('status').textContent, /No se pudo actualizar/);
    assert.match(view.get('status').textContent, /desactualizados/);
});

test('disabled Steam and missing channel do not pretend login is available', async () => {
    const data = fixture();
    data.steam_enabled = false;
    const view = await app({ data });
    assert.equal(view.get('steam-login').hidden, true);
    assert.match(view.get('account-status').textContent, /STEAM_WEB_API_KEY/);
    const missing = await app({ search: '' });
    assert.equal(missing.requests.length, 0);
    assert.match(missing.get('status').textContent, /Falta un canal válido/);
});

test('private panel token is removed from URL and only used for management', async () => {
    const view = await app({ hash: '#panel=panel-secret' });
    assert.equal(view.history[0][2], '/l4d2.html?ch=alpha');
    assert.equal(view.get('admin-controls').hidden, false);
    assert.equal(view.requests.find(request => request.path.endsWith('/admin')).options.headers.Authorization, 'Bearer panel-secret');
    view.get('player-name').value = '<img src=x>';
    view.get('steam-profile').value = '76561197960265729';
    await view.get('link-form').listeners.submit({ preventDefault() {} });
    const post = view.requests.find(request => request.options.method === 'POST');
    assert.deepEqual(JSON.parse(post.options.body), { action: 'link', name: '<img src=x>', steamid: '76561197960265729' });
    assert.equal(post.options.headers.Authorization, 'Bearer panel-secret');
});

test('overlay requests no web session and selected Steam statistic is included in the linked row', async () => {
    const view = await app({ search: '?ch=alpha&view=overlay&stat=kills' });
    assert.equal(view.context.document.documentElement.classList.contains('obs'), true);
    assert.ok(view.requests.every(request => !request.path.endsWith('/account')));
    assert.match(view.get('players').textContent, /kills: 123/);
    assert.equal(view.get('profiles').children.length, 0);
});

test('account removal calls the authenticated endpoint and public view reports missing history', async () => {
    const data = fixture();
    data.profiles[0].profile = { stats_error: 'Perfil privado' };
    const view = await app({ data, me: '76561197960265729' });
    assert.equal(view.get('remove-account').hidden, false);
    assert.match(view.get('players').textContent, /Histórico no disponible/);
    assert.match(view.get('profiles').textContent, /Perfil privado/);
    await view.get('remove-account').listeners.click();
    assert.ok(view.requests.some(request => request.path.endsWith('/account') && request.options.method === 'DELETE'));
    assert.equal(view.get('remove-account').hidden, true);
});
