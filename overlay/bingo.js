(() => {
    'use strict';
    const el = id => document.getElementById(id);
    const params = new URLSearchParams(location.hash.slice(1));
    const channel = params.get('ch') || '';
    const token = params.get('token') || '';
    const letters = ['B', 'I', 'N', 'G', 'O'];
    const storageKey = `bingo-marks:${channel}:${token}`;
    let marks = new Set(), snapshot = null, cells = [], stopped = false;
    const storageError = () => { el('storage').textContent = 'El navegador no permite guardar las marcas. Se conservarán solo mientras esta página esté abierta.'; };

    function save() {
        try { localStorage.setItem(storageKey, JSON.stringify([...marks])); }
        catch { storageError(); }
    }

    function paint() {
        if (!snapshot) return;
        const drawn = new Set(snapshot.drawn);
        cells.forEach((button, index) => {
            const number = snapshot.card[index];
            const marked = number === 0 || marks.has(index);
            button.classList.toggle('marked', marked);
            button.classList.toggle('drawn', number !== 0 && drawn.has(number));
            button.setAttribute('aria-pressed', String(marked));
            button.disabled = number === 0 || stopped || snapshot.phase === 'won';
        });
    }

    function render(data) {
        if (!Array.isArray(data.card) || data.card.length !== 25 || data.card[12] !== 0
            || !data.card.every((n, i) => Number.isInteger(n) && (i === 12 || (n >= Math.floor(i % 5) * 15 + 1 && n <= (i % 5 + 1) * 15)))
            || !Array.isArray(data.drawn) || !data.drawn.every(n => Number.isInteger(n) && n >= 1 && n <= 75)
            || !['open', 'running', 'exhausted', 'won'].includes(data.phase)) {
            throw new Error('El servidor devolvió un cartón inválido. Vuelve a solicitar !carton.');
        }
        snapshot = data;
        el('title').textContent = `Cartón de ${data.user}`;
        el('status').textContent = {
            open: 'Inscripciones abiertas · esperando el inicio',
            running: `Partida en curso · ${data.drawn.length} de 75 bolas`,
            exhausted: 'Salieron las 75 bolas · aún puedes reclamar',
            won: `Partida terminada · ganador: ${data.winner}`
        }[data.phase];
        if (!cells.length) {
            cells = data.card.map((number, index) => {
                const button = document.createElement('button');
                button.type = 'button';
                button.className = `cell${number === 0 ? ' free' : ''}`;
                button.textContent = number === 0 ? 'LIBRE' : String(number);
                button.setAttribute('aria-label', `Fila ${Math.floor(index / 5) + 1}, ${number === 0 ? 'centro libre' : letters[index % 5] + number}`);
                button.addEventListener('click', () => {
                    if (number === 0 || stopped || snapshot.phase === 'won') return;
                    if (marks.has(index)) marks.delete(index); else marks.add(index);
                    save();
                    paint();
                });
                return button;
            });
            el('card').replaceChildren(...cells);
        }
        const label = number => `${letters[Math.floor((number - 1) / 15)]}${number}`;
        el('last').textContent = data.drawn.length ? label(data.drawn.at(-1)) : '—';
        el('drawn').textContent = data.drawn.length ? data.drawn.map(label).join(' · ') : 'Esperando el sorteo.';
        el('claim').textContent = data.ready
            ? '¡Tienes una línea con las bolas sorteadas! Escribe !bingo en el chat del stream para reclamar.'
            : 'Completa una fila, columna o diagonal y escribe !bingo en el chat del stream.';
        el('claim').classList.toggle('ready', data.ready === true);
        paint();
        if (data.phase === 'won') stopped = true;
    }

    async function refresh() {
        try {
            const response = await fetch(`/api/bingo/${encodeURIComponent(channel)}`, {
                headers: { 'x-bingo-token': token }, cache: 'no-store', credentials: 'same-origin'
            });
            if (response.status === 401 || response.status === 403) {
                stopped = true;
                snapshot = null;
                cells = [];
                el('card').replaceChildren();
                el('title').textContent = 'Tu cartón personal';
                el('last').textContent = '—';
                el('drawn').textContent = 'Inicia sesión para consultar las bolas.';
                el('claim').textContent = 'Tu cartón solo se muestra a su dueño.';
                el('claim').classList.toggle('ready', false);
                el('login').hidden = false;
                el('status').textContent = 'Acceso privado con Kick';
                throw new Error(response.status === 401
                    ? 'Inicia sesión con la cuenta de Kick con la que solicitaste !carton.'
                    : 'Este cartón pertenece a otra cuenta. Cambia de cuenta en Kick y vuelve a iniciar sesión.');
            }
            if (response.status === 404) {
                stopped = true;
                el('status').textContent = 'Cartón no disponible';
                throw new Error('El enlace expiró o la partida fue cancelada. Escribe !carton en el chat para obtener uno válido.');
            }
            if (!response.ok) throw new Error('No se pudo consultar la partida. Reintentando...');
            render(await response.json());
            el('error').textContent = '';
        } catch (error) {
            el('error').textContent = error.message || 'Sin conexión. Reintentando...';
            if (!stopped) el('status').textContent = 'Conexión interrumpida · los datos pueden estar desactualizados';
            paint();
        }
        if (!stopped) setTimeout(refresh, 3000);
    }

    if (!/^[a-z0-9_-]{1,100}$/i.test(channel) || !/^[a-f0-9]{32}$/i.test(token)) {
        el('status').textContent = 'Necesitas tu enlace personal';
        el('error').textContent = 'Escribe !carton en el chat del stream y abre el enlace que responde el bot.';
        return;
    }
    try {
        const saved = JSON.parse(localStorage.getItem(storageKey) || '[]');
        if (!Array.isArray(saved) || !saved.every(i => Number.isInteger(i) && i >= 0 && i < 25)) {
            throw new Error('Las marcas guardadas no son válidas.');
        }
        marks = new Set(saved);
    } catch { storageError(); }
    el('chat').href = `https://kick.com/${encodeURIComponent(channel)}`;
    el('chat').hidden = false;
    el('login').href = `/auth/bingo?${new URLSearchParams({ ch: channel, token })}`;
    refresh();
})();
