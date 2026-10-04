pub const VOICE_ALIAS: &str = "jacinta";
pub const MODEL_ID: &str = "72649a85fadd44a585569237c03fffd1";
const TTS_MODEL: &str = "s2.1-pro-free";
const TTS_ENDPOINT: &str = "https://api.fish.audio/v1/tts";
const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;

pub async fn synthesize(
    http: &reqwest::Client,
    api_key: &str,
    text: &str,
) -> Result<Vec<u8>, String> {
    if api_key.is_empty() {
        return Err("FISH_AUDIO_API_KEY no está configurada".to_string());
    }

    let mut response = http
        .post(TTS_ENDPOINT)
        .bearer_auth(api_key)
        .header("model", TTS_MODEL)
        .json(&serde_json::json!({
            "text": text,
            "reference_id": MODEL_ID,
            "format": "mp3"
        }))
        .send()
        .await
        .map_err(|error| format!("Fish Audio no disponible: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Fish Audio respondió con estado {}",
            response.status()
        ));
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUDIO_BYTES as u64)
    {
        return Err("Audio de Fish Audio demasiado grande".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Fish Audio no pudo leer el audio: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_AUDIO_BYTES {
            return Err("Audio de Fish Audio demasiado grande".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("Fish Audio devolvió audio vacío".to_string());
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fish_voice_alias_and_model_are_registered() {
        assert_eq!(VOICE_ALIAS, "jacinta");
        assert_eq!(MODEL_ID, "72649a85fadd44a585569237c03fffd1");
        assert!(crate::tts::is_valid_voice(VOICE_ALIAS));
    }
}
