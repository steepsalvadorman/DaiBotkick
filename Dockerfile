# ── Stage 1: compilar el bot en Rust ─────────────────────────────────────────
FROM rust:1.82-bookworm AS builder
WORKDIR /build
COPY backend/ .
RUN cargo build --release

# ── Stage 2: imagen de producción ─────────────────────────────────────────────
FROM debian:bookworm-slim

# Dependencias del sistema + Python + edge-tts para TTS
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        python3 \
        python3-pip \
    && pip3 install edge-tts --break-system-packages \
    && rm -rf /var/lib/apt/lists/*

# Copiar el bot compilado y el overlay
COPY --from=builder /build/target/release/daibot /app/daibot
COPY overlay/ /app/overlay/

# Crear directorios de datos
RUN mkdir -p /app/data/tts_cache

WORKDIR /app

# Variables de entorno por defecto (se sobreescriben en el dashboard de Render)
ENV OVERLAY_DIR=/app/overlay
ENV QUEUE_FILE=/app/data/queue.json
ENV TTS_CACHE_DIR=/tmp/tts_cache

EXPOSE 3000
CMD ["/app/daibot"]
