// Utilidades compartidas por los docks de OBS. El token de panel viaja en el fragmento (#token=…)
// para que no aparezca en logs ni en referers; se envía como `Authorization: Bearer`.
(function () {
    const params = new URLSearchParams(location.search);
    const hash = new URLSearchParams(location.hash.slice(1));
    const channel = (params.get('ch') || '').trim().toLowerCase();
    const token = hash.get('token') || '';
    const $ = id => document.getElementById(id);

    function notice(text, error) {
        const box = $('notice'); if (!box) return;
        box.textContent = text || '';
        box.classList.toggle('error', Boolean(error));
    }

    async function api(path, options = {}) {
        const response = await fetch(`/api/dock/${encodeURIComponent(channel)}/${path}`, {
            ...options,
            headers: { Authorization: `Bearer ${token}`, ...(options.body ? { 'Content-Type': 'application/json' } : {}) },
            cache: 'no-store',
        });
        if (response.status === 204) return null;
        let data = null;
        try { data = await response.json(); } catch { /* respuesta sin JSON */ }
        if (!response.ok) throw new Error(data?.error || `Error ${response.status}`);
        return data;
    }

    function renderContent(target, content) {
        const pattern = /\[emote:(\d{1,20}):([^\]\r\n]{1,100})\]/g;
        let offset = 0;
        for (const match of content.matchAll(pattern)) {
            target.append(document.createTextNode(content.slice(offset, match.index)));
            const image = document.createElement('img');
            image.className = 'emote'; image.alt = match[2]; image.title = match[2];
            image.src = `https://files.kick.com/emotes/${match[1]}/fullsize`;
            image.referrerPolicy = 'no-referrer'; image.loading = 'lazy';
            image.addEventListener('error', () => image.replaceWith(document.createTextNode(`:${match[2]}:`)), { once: true });
            target.append(image);
            offset = match.index + match[0].length;
        }
        target.append(document.createTextNode(content.slice(offset)));
    }

    window.Dock = { channel, token, $, notice, api, renderContent, ready: Boolean(channel && token) };
    if (!window.Dock.ready) notice('Falta el canal o el token. Copia la URL del dock desde la página de conexión de DaiBot.', true);
    if ($('channel')) $('channel').textContent = channel || 'sin canal';
})();
