(() => {
    'use strict';
    const el = id => document.getElementById(id);
    const params = new URLSearchParams(location.search);
    const channel = params.get('ch') || '';
    const obs = params.get('view') === 'overlay';
    document.documentElement.classList.toggle('obs', obs);
    let token = new URLSearchParams(location.hash.slice(1)).get('panel') || '';
    if (location.hash) history.replaceState(null, '', location.pathname + location.search);
    el('panel-token').value = token;
    const endpoint = `/api/l4d2/${encodeURIComponent(channel)}`;
    let data = null, me = null, unlocked = false;
    let featuredStat = params.get('stat') || '';
    const number = value => new Intl.NumberFormat('es', { maximumFractionDigits: 1 }).format(value);
    function node(tag, text, className) {
        const result = document.createElement(tag);
        if (text !== undefined) result.textContent = text;
        if (className) result.className = className;
        return result;
    }
    function message(id, text, error = false) {
        el(id).textContent = text;
        el(id).setAttribute('data-error', String(error));
    }
    async function request(path, options = {}) {
        const response = await fetch(path, { credentials: 'same-origin', cache: 'no-store', ...options });
        if (!response.ok) throw new Error((await response.text()).slice(0, 500) || `Error HTTP ${response.status}`);
        return response.status === 204 ? null : response.json();
    }
    function age(timestamp) {
        return Number.isFinite(timestamp) ? Math.max(0, Date.now() / 1000 - timestamp) : Infinity;
    }
    function avatar(url) {
        const image = node('img');
        image.alt = '';
        image.referrerPolicy = 'no-referrer';
        image.src = '/daibot.svg';
        if (typeof url === 'string' && url) {
            if (/^https:\/\/(?:[a-z0-9-]+\.)*(?:steamstatic\.com|steamcommunity\.com|akamaihd\.net)\/[^\s"'<>]*$/i.test(url)) image.src = url;
            else message('status', 'Steam devolvió una URL de avatar no permitida. Se muestra la mascota local.', true);
        }
        image.addEventListener('error', () => {
            image.removeAttribute('src');
            image.hidden = true;
            message('status', 'No se pudo cargar un avatar de Steam. Los datos del perfil siguen disponibles.', true);
        }, { once: true });
        return image;
    }
    function historySummary(profile) {
        const values = [];
        if (Number.isFinite(profile.minutes)) values.push(`${number(profile.minutes / 60)} h en L4D2`);
        if (profile.achievements) values.push(`${profile.achievements.unlocked}/${profile.achievements.total} logros`);
        if (featuredStat) {
            const stat = profile.stats?.find(stat => stat.name === featuredStat);
            values.push(stat ? `${stat.name}: ${number(stat.value)}` : `${featuredStat}: no disponible`);
        }
        return values.join(' · ') || 'Histórico no disponible';
    }
    function selectOptions(id, options) {
        const select = el(id), previous = select.value;
        select.replaceChildren(...options.map(([value, text]) => {
            const option = node('option', text);
            option.value = value;
            return option;
        }));
        if (options.some(([value]) => value === previous)) select.value = previous;
    }
    function renderPlayers() {
        const snapshot = data.snapshot;
        const server = snapshot.server;
        const stale = age(snapshot.updated) > 90;
        el('freshness').textContent = snapshot.updated
            ? `${stale ? 'Datos antiguos · ' : ''}${new Date(snapshot.updated * 1000).toLocaleTimeString('es')}`
            : 'Sin consulta';
        el('server-description').textContent = server
            ? `${server.name} · ${server.map} · ${server.online}/${server.capacity} jugadores según A2S`
            : 'Sin datos actuales del servidor.';
        const players = server?.players || [];
        el('players').replaceChildren();
        if (!players.length) {
            const row = node('tr'), cell = node('td', server ? 'El servidor no expone jugadores en A2S_PLAYER o está vacío.' : 'Configura el servidor o revisa el aviso de conexión.');
            cell.colSpan = 4; row.append(cell); el('players').append(row);
        }
        for (const player of players) {
            const row = node('tr');
            row.append(node('td', player.name || 'Sin nombre (A2S)'),
                node('td', number(player.score)), node('td', `${number(player.seconds / 60)} min`));
            const cell = node('td');
            const unique = players.filter(p => p.name === player.name).length === 1;
            const link = unique && data.links.find(link => link.name === player.name);
            const account = link && data.profiles.find(profile => profile.steamid === link.steamid);
            if (account) {
                const identity = node('div', undefined, 'identity'), copy = node('div');
                copy.append(node('strong', account.profile.name || account.steamid),
                    node('small', 'Asociación manual · no verificada', 'manual'),
                    node('small', historySummary(account.profile)));
                if (account.profile.error || account.profile.stats_error) copy.append(node('small', 'Estadísticas no disponibles'));
                if (age(account.updated) > 900) copy.append(node('small', 'Histórico pendiente de actualizar'));
                identity.append(avatar(account.profile.avatar), copy); cell.append(identity);
            } else cell.textContent = unique ? 'Sin asociación Steam' : 'Nombre duplicado · identidad ambigua';
            row.append(cell); el('players').append(row);
        }
        selectOptions('player-name', players.filter(p => p.name && players.filter(other => other.name === p.name).length === 1)
            .map(p => [p.name, p.name]));
        selectOptions('steam-profile', data.profiles.map(p => [p.steamid, `${p.profile.name || p.steamid} · ${p.steamid}`]));
        const steamOnly = obs && !players.length;
        if (obs) renderObsProfiles(steamOnly);
        if (snapshot.error) message('status', snapshot.error, true);
        else if (stale && snapshot.updated) message('status', 'La consulta del servidor está atrasada. No se garantiza que estos jugadores sigan conectados.', true);
        else if (steamOnly && !server) message('status', 'Sin servidor en vivo: estadísticas históricas de Steam del canal.');
        else message('status', server ? 'Consulta A2S recibida. Los perfiles Steam se asocian manualmente.'
            : obs ? 'Esperando servidor.' : 'Esperando servidor. Puedes configurarlo más abajo con tu token de panel.');
    }
    // OBS without live players: show the channel's Steam profiles instead of an empty table.
    function renderObsProfiles(show) {
        const box = el('obs-profiles');
        box.hidden = !show;
        el('players-table').hidden = show;
        el('a2s-notice').hidden = show;
        el('server-title').textContent = show ? 'Left 4 Dead 2 · Steam' : 'Servidor de la partida';
        box.replaceChildren();
        if (!show) return;
        if (!data.profiles.length) {
            box.append(node('p', 'Inicia sesión con Steam en el panel de L4D2 para mostrar tus estadísticas aquí.'));
            return;
        }
        for (const account of data.profiles) {
            const card = node('div', undefined, 'identity'), copy = node('div');
            copy.append(node('strong', account.profile.name || account.steamid), node('small', historySummary(account.profile)));
            if (account.profile.error || account.profile.stats_error) copy.append(node('small', 'Estadísticas no disponibles'));
            card.append(avatar(account.profile.avatar), copy);
            box.append(card);
        }
    }
    function renderProfiles() {
        const profiles = el('profiles');
        profiles.replaceChildren();
        if (!data.profiles.length) profiles.append(node('p', 'Todavía no hay perfiles. Inicia sesión con Steam para compartir el tuyo.'));
        for (const account of data.profiles) {
            const profile = account.profile, section = node('article', undefined, 'profile');
            const heading = node('div', undefined, 'profile-heading');
            const link = node('a', profile.name || account.steamid);
            link.href = `https://steamcommunity.com/profiles/${encodeURIComponent(account.steamid)}/`;
            link.target = '_blank'; link.rel = 'noopener noreferrer';
            heading.append(avatar(profile.avatar), link); section.append(heading);
            section.append(node('p', `${historySummary(profile)} · Actualizado ${new Date(account.updated * 1000).toLocaleString('es')}`));
            for (const key of ['error', 'stats_error', 'achievements_error', 'playtime_error']) {
                if (profile[key]) section.append(node('p', profile[key]));
            }
            if (profile.stats?.length) {
                const details = node('details'), list = node('dl');
                details.append(node('summary', `Ver ${profile.stats.length} estadísticas acumuladas de L4D2`));
                for (const stat of profile.stats) list.append(node('dt', stat.name), node('dd', number(stat.value)));
                details.append(list); section.append(details);
            }
            profiles.append(section);
        }
        const names = [...new Set(data.profiles.flatMap(p => (p.profile.stats || []).map(s => s.name)))].sort();
        if (featuredStat && !names.includes(featuredStat)) names.push(featuredStat);
        selectOptions('featured-stat', [['', 'Solo horas y logros'], ...names.map(name => [name, name])]);
        el('featured-stat').value = featuredStat;
        updateObsLink();
    }
    function updateObsLink() {
        const query = new URLSearchParams({ ch: channel, view: 'overlay' });
        if (featuredStat) query.set('stat', featuredStat);
        el('obs-link').href = `/l4d2.html?${query}`;
    }
    el('featured-stat').addEventListener('change', () => {
        featuredStat = el('featured-stat').value;
        if (data) { renderPlayers(); renderProfiles(); }
    });
    async function refresh() {
        data = await request(endpoint);
        renderPlayers();
        if (!obs) renderProfiles();
        el('steam-login').hidden = !data.steam_enabled || Boolean(me);
        el('remove-account').hidden = !me;
        message('account-status', me ? `Sesión Steam: ${me}. Puedes retirar tu perfil y sus asociaciones de este canal.`
            : data.steam_enabled ? 'Tu contraseña solo se introduce en steamcommunity.com.' : 'Steam está desactivado: el administrador debe configurar STEAM_WEB_API_KEY.');
    }
    async function unlock() {
        const config = await request(endpoint + '/admin', { headers: { Authorization: `Bearer ${token}` } });
        selectOptions('server-address', [['', 'Desactivar consulta A2S'], ...config.allowed_servers.map(s => [s, s])]);
        el('server-address').value = config.server || '';
        el('admin-controls').hidden = false;
        unlocked = true;
        message('admin-status', config.allowed_servers.length ? 'Acceso autorizado.' : 'No hay servidores permitidos. Configura STEAM_A2S_SERVERS en el backend.');
    }
    async function change(body) {
        if (!unlocked) throw new Error('Accede primero con tu token de panel.');
        await request(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` }, body: JSON.stringify(body) });
        message('admin-status', 'Cambio guardado. La consulta del servidor puede tardar hasta el próximo ciclo.');
        await refresh();
    }
    function submit(id, action) {
        el(id).addEventListener('submit', async event => {
            event.preventDefault();
            const buttons = [...el(id).querySelectorAll('button')];
            buttons.forEach(button => { button.disabled = true; });
            try { await action(); }
            catch (error) { message('admin-status', error.message, true); }
            finally { buttons.forEach(button => { button.disabled = false; }); }
        });
    }
    submit('unlock-form', async () => {
        token = el('panel-token').value.trim();
        unlocked = false;
        el('admin-controls').hidden = true;
        await unlock();
    });
    submit('server-form', () => change({ action: 'server', address: el('server-address').value }));
    submit('link-form', () => change({ action: 'link', name: el('player-name').value, steamid: el('steam-profile').value }));
    el('unlink-player').addEventListener('click', async () => {
        try { await change({ action: 'unlink', name: el('player-name').value }); }
        catch (error) { message('admin-status', error.message, true); }
    });
    el('remove-account').addEventListener('click', async () => {
        el('remove-account').disabled = true;
        try {
            await request(endpoint + '/account', { method: 'DELETE' });
            me = null; await refresh();
        } catch (error) { message('account-status', error.message, true); }
        finally { el('remove-account').disabled = false; }
    });
    async function start() {
        if (!/^[a-zA-Z0-9_-]{1,100}$/.test(channel)) {
            message('status', 'Falta un canal válido. Abre /l4d2.html?ch=tu_canal.', true);
            el('steam-login').hidden = true; return;
        }
        el('channel-name').textContent = `Canal ${channel}`;
        el('steam-login').href = `/auth/steam?${new URLSearchParams({ ch: channel })}`;
        el('obs-link').href = `/l4d2.html?${new URLSearchParams({ ch: channel, view: 'overlay' })}`;
        try {
            if (!obs) me = (await request(endpoint + '/account')).steamid;
            await refresh();
            if (token && !obs) await unlock();
        } catch (error) { message('status', error.message, true); }
        async function poll() {
            try { await refresh(); }
            catch (error) { message('status', `No se pudo actualizar: ${error.message}. Los datos visibles pueden estar desactualizados.`, true); }
            setTimeout(poll, 30000);
        }
        setTimeout(poll, 30000);
    }
    start();
})();
