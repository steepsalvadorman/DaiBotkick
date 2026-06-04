// Usa el CLI de edge-tts (Python) como subprocess.
// Más confiable que reimplementar el protocolo WebSocket de Microsoft,
// que cambia su autenticación frecuentemente.

use uuid::Uuid;

pub const VOICES: &[(&str, &str)] = &[
    ("dalia",   "es-MX-DaliaNeural"),
    ("jorge",   "es-MX-JorgeNeural"),
    ("camila",  "es-PE-CamilaNeural"),
    ("alex",    "es-PE-AlexNeural"),
    ("jacinta", "es-PE-CamilaNeural"),
];

pub fn voice_name(id: &str) -> &'static str {
    VOICES.iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
        .unwrap_or("es-PE-CamilaNeural")
}

pub fn is_valid_voice(id: &str) -> bool {
    VOICES.iter().any(|(k, _)| *k == id)
}

/// Busca edge-tts: primero en el Python bundled junto al exe, luego en el PATH.
fn make_edge_tts_cmd() -> tokio::process::Command {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(app) = exe.parent() {
            // Python portable bundled: {app}/python/Scripts/edge-tts.exe
            let bundled = app.join("python").join("Scripts").join("edge-tts.exe");
            if bundled.exists() {
                return tokio::process::Command::new(bundled);
            }
            // Alternativa: llamar al módulo directamente con python.exe bundled
            let python = app.join("python").join("python.exe");
            if python.exists() {
                let mut cmd = tokio::process::Command::new(python);
                cmd.args(["-m", "edge_tts"]);
                return cmd;
            }
        }
    }
    // Fallback: edge-tts en el PATH del sistema
    tokio::process::Command::new("edge-tts")
}

/// Sintetiza texto usando el CLI `edge-tts` de Python y devuelve bytes MP3.
pub async fn synthesize(text: &str, voice_id: &str) -> Result<Vec<u8>, String> {
    let vname = voice_name(voice_id);
    let tmp = std::env::temp_dir().join(format!("tts_{}.mp3", Uuid::new_v4()));

    let mut cmd = make_edge_tts_cmd();
    cmd.arg("--voice").arg(vname)
       .arg("--text").arg(text)
       .arg("--write-media").arg(&tmp);

    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let out = cmd.output().await
        .map_err(|e| format!("edge-tts spawn: {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("edge-tts: {err}"));
    }

    let bytes = tokio::fs::read(&tmp).await
        .map_err(|e| format!("edge-tts leer salida: {e}"))?;

    let _ = tokio::fs::remove_file(&tmp).await;

    if bytes.is_empty() {
        return Err("edge-tts: archivo de salida vacío".to_string());
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_voices_resolve_correctly() {
        assert_eq!(voice_name("camila"),  "es-PE-CamilaNeural");
        assert_eq!(voice_name("dalia"),   "es-MX-DaliaNeural");
        assert_eq!(voice_name("jorge"),   "es-MX-JorgeNeural");
        assert_eq!(voice_name("alex"),    "es-PE-AlexNeural");
        assert_eq!(voice_name("jacinta"), "es-PE-CamilaNeural");
    }

    #[test]
    fn unknown_voice_falls_back_to_camila() {
        assert_eq!(voice_name("noexiste"), "es-PE-CamilaNeural");
        assert_eq!(voice_name(""),         "es-PE-CamilaNeural");
    }

    #[test]
    fn all_voices_in_list_are_valid() {
        for (name, _) in VOICES {
            assert!(is_valid_voice(name), "'{name}' debería ser válida");
        }
    }

    #[test]
    fn unknown_voices_are_invalid() {
        assert!(!is_valid_voice(""));
        assert!(!is_valid_voice("s"));
        assert!(!is_valid_voice("español"));
        assert!(!is_valid_voice("es-PE-CamilaNeural")); // nombre completo, no alias
    }
}
