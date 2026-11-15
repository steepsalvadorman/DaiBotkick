//! Salida al chat: cada canal responde en su propia plataforma.

use crate::{
    platform::Platform,
    state::{AppState, ChannelState},
};
use std::sync::Arc;

/// Publica `text` en el chat del canal como el bot de su plataforma.
pub async fn send(text: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    match ch.platform {
        Platform::Kick => crate::kick::sender::send(text, ch, global).await,
        // El emisor de Twitch llega en la etapa 5 del plan.
        Platform::Twitch => {
            tracing::warn!("[Chat][{}] Twitch aún no puede enviar mensajes", ch.slug)
        }
    }
}
