FROM rust:1.88.0-bookworm AS build
WORKDIR /src
COPY backend/ .
RUN cargo build --release --locked

FROM python:3.12-slim-bookworm
RUN pip install --no-cache-dir edge-tts==7.3.1 yt-dlp==2026.8.19 \
    && useradd --system --uid 10001 daibot
WORKDIR /app
COPY --from=build /src/target/release/daibot ./daibot
COPY overlay/ ./overlay/
COPY comandos.html ./overlay/
ENV OVERLAY_DIR=/app/overlay TTS_CACHE_DIR=/tmp/daibot_tts PORT=3000
USER daibot
EXPOSE 3000
CMD ["./daibot"]
