// Utilidades compartidas por los docks de OBS. El token de panel viaja en el fragmento (#token=…)
// para que no aparezca en logs ni en referers; se envía como `Authorization: Bearer`.
(function () {
    const params = new URLSearchParams(location.search);
    const hash = new URLSearchParams(location.hash.slice(1));
    // ?ch=slug es un canal de Kick; ?tw=login, uno de Twitch.
    const twitch = (params.get('tw') || '').trim().toLowerCase();
    const channel = twitch || (params.get('ch') || '').trim().toLowerCase();
    const platformName = twitch ? 'Twitch' : 'Kick';
    // Clave del canal en el servidor: el slug en Kick, `twitch:login` en Twitch.
    const key = twitch ? `twitch:${twitch}` : channel;
    const socketQuery = twitch ? { tw: twitch } : { ch: channel };
    const token = hash.get('token') || '';
    const $ = id => document.getElementById(id);

    function notice(text, error) {
        const box = $('notice'); if (!box) return;
        box.textContent = text || '';
        box.classList.toggle('error', Boolean(error));
    }

    async function api(path, options = {}) {
        const response = await fetch(`/api/dock/${encodeURIComponent(key)}/${path}`, {
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

    function emote(name, src) {
        const image = document.createElement('img');
        image.className = 'emote'; image.alt = name; image.title = name;
        image.src = src; image.referrerPolicy = 'no-referrer'; image.loading = 'lazy';
        image.addEventListener('error', () => image.replaceWith(document.createTextNode(`:${name}:`)), { once: true });
        return image;
    }

    // Twitch manda el mensaje en partes (texto y emotes por ID); la URL se arma aquí, solo en su CDN.
    function renderContent(target, content, parts) {
        if (Array.isArray(parts) && parts.length) {
            for (const part of parts.slice(0, 200)) {
                if (part?.type === 'emote' && /^[A-Za-z0-9_]{1,80}$/.test(String(part.id))) {
                    target.append(emote(String(part.name || '').slice(0, 100), `https://static-cdn.jtvnw.net/emoticons/v2/${part.id}/default/dark/2.0`));
                } else {
                    target.append(document.createTextNode(String(part?.text ?? part?.name ?? '')));
                }
            }
            return;
        }
        const pattern = /\[emote:(\d{1,20}):([^\]\r\n]{1,100})\]/g;
        let offset = 0;
        for (const match of content.matchAll(pattern)) {
            target.append(document.createTextNode(content.slice(offset, match.index)));
            target.append(emote(match[2], `https://files.kick.com/emotes/${match[1]}/fullsize`));
            offset = match.index + match[0].length;
        }
        target.append(document.createTextNode(content.slice(offset)));
    }

    window.Dock = { channel, token, platformName, socketQuery, isTwitch: Boolean(twitch), $, notice, api, renderContent, ready: Boolean(channel && token) };
    if (!window.Dock.ready) notice('Falta el canal o el token. Copia la URL del dock desde la página de conexión de DaiBot.', true);
    if ($('channel')) $('channel').textContent = channel || 'sin canal';
})();
