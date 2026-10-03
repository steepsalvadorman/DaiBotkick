const test = require('node:test');
const assert = require('node:assert/strict');
const base = process.env.DAIBOT_BASE_URL || 'http://127.0.0.1:3000';

async function get(path) {
    return fetch(new URL(path, base), { signal: AbortSignal.timeout(5000) });
}

test('the running backend is live and database-ready', async () => {
    for (const path of ['/healthz', '/readyz']) {
        assert.equal((await get(path)).status, 200, path);
    }
});

test('metrics expose process and durable inbox counters', async () => {
    const response = await get('/metrics');
    assert.equal(response.status, 200);
    const body = await response.text();
    for (const metric of ['channels', 'sockets', 'tts_pending', 'webhook_received_total', 'webhook_processed_total']) {
        assert.match(body, new RegExp(`^daibot_${metric} \\d+$`, 'm'));
    }
});

test('the packaged overlay, assets and command reference are served', async () => {
    for (const [path, content] of [
        ['/pixel.html', /app\.js/],
        ['/app.js', /syncQueue/],
        ['/style.css', /media-widget/],
        ['/sio.js', /Socket\.IO/i],
        ['/comandos.html', /!play/],
    ]) {
        const response = await get(path);
        assert.equal(response.status, 200, path);
        assert.match(await response.text(), content, path);
        assert.equal(response.headers.get('x-content-type-options'), 'nosniff');
    }
});

test('index retains the query and fragment when it redirects', async () => {
    const response = await get('/index.html?ch=alpha');
    assert.equal(response.status, 200);
    assert.match(await response.text(), /location\.search\s*\+\s*location\.hash/);
});
