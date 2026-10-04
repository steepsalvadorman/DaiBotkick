FROM rust:1.88.0-bookworm AS builder
WORKDIR /build
COPY backend/ .
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates python3 python3-venv \
    && python3 -m venv /opt/venv \
    && /opt/venv/bin/pip install --no-cache-dir edge-tts==7.2.8 yt-dlp==2026.8.19 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home daibot

COPY --from=builder /build/target/release/daibot /app/daibot
COPY overlay/ /app/overlay/
COPY comandos.html /app/overlay/comandos.html
WORKDIR /app
ENV PATH="/opt/venv/bin:$PATH" \
    OVERLAY_DIR=/app/overlay \
    TTS_CACHE_DIR=/tmp/daibot_tts \
    PORT=3000
USER daibot
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD python3 -c "import os, urllib.request; urllib.request.urlopen('http://127.0.0.1:' + os.environ.get('PORT', '3000') + '/readyz', timeout=3)"
CMD ["/app/daibot"]
