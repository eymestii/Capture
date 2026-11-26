use capture_core::{AudioEncoderKind, Config, OutputProtocol, VideoEncoderKind};
use serde_json::Value;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("capture")
        .join("config.json")
}

pub fn load() -> Config {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Config::default();
    };
    if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
        return cfg;
    }
    // Formato anterior: `{ camera, screen, live }`. Se migra en memoria y se
    // reescribe en el formato nuevo al primer guardado.
    from_legacy(&text).unwrap_or_default()
}

pub fn save(c: &Config) {
    if let Some(parent) = path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(path(), s);
    }
}

// ---------- migración del formato viejo ----------

fn text<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(|x| x.as_str())
}

fn number(v: &Value, ptr: &str) -> Option<u32> {
    v.pointer(ptr).and_then(|x| x.as_u64()).map(|n| n as u32)
}

fn video_kind(s: &str) -> Option<VideoEncoderKind> {
    match s {
        "NvencH264" => Some(VideoEncoderKind::NvencH264),
        "NvencHevc" => Some(VideoEncoderKind::NvencHevc),
        "X264" => Some(VideoEncoderKind::X264),
        _ => None,
    }
}

fn audio_kind(s: &str) -> Option<AudioEncoderKind> {
    match s {
        "Opus" => Some(AudioEncoderKind::Opus),
        "Aac" => Some(AudioEncoderKind::Aac),
        _ => None,
    }
}

fn from_legacy(texto: &str) -> Option<Config> {
    let v: Value = serde_json::from_str(texto).ok()?;
    // Si ya trae la clave del formato nuevo, no es legacy.
    if v.get("camera_device").is_some() || v.get("live").is_none() {
        return None;
    }

    let mut cfg = Config::default();

    if let Some(s) = text(&v, "/camera/video/device") {
        cfg.camera_device = s.to_string();
    }
    if let Some(s) = text(&v, "/camera/audio/device") {
        cfg.microphone_device = s.to_string();
    }
    if let Some(s) = text(&v, "/live/encoder/video").and_then(video_kind) {
        cfg.video.kind = s;
    }
    if let Some(n) = number(&v, "/live/encoder/video_bitrate_kbps") {
        cfg.video.bitrate_kbps = n;
    }
    if let Some(n) = number(&v, "/live/encoder/keyframe_interval") {
        cfg.video.keyframe_interval = n;
    }
    if let Some(s) = text(&v, "/live/encoder/audio").and_then(audio_kind) {
        cfg.audio.kind = s;
    }
    if let Some(n) = number(&v, "/live/audio/bitrate_kbps") {
        cfg.audio.bitrate_kbps = n;
    }
    if let Some(s) = text(&v, "/live/output/protocol") {
        cfg.output.protocol = match s {
            "Rtmp" => OutputProtocol::Rtmp,
            _ => OutputProtocol::Srt,
        };
    }
    if let Some(s) = text(&v, "/live/output/uri") {
        cfg.output.uri = s.to_string();
    }
    if let Some(n) = number(&v, "/live/output/srt_latency_ms") {
        cfg.output.srt_latency_ms = n;
    }
    if let Some(s) = text(&v, "/live/output/srt_passphrase") {
        if !s.is_empty() {
            cfg.output.srt_passphrase = Some(s.to_string());
        }
    }

    Some(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = r#"{
      "camera": {
        "name": "Cámara",
        "video": { "source": "Camera", "device": "/dev/video2", "width": 1280, "height": 720, "fps": 30 },
        "audio": { "source": "Microphone", "device": "alsa_input.usb", "bitrate_kbps": 128 },
        "encoder": { "video": "NvencHevc", "video_bitrate_kbps": 5000, "keyframe_interval": 90, "quality_preset": "default", "audio": "Aac" },
        "output": { "protocol": "Srt", "uri": "srt://1.2.3.4:9000?mode=caller", "srt_latency_ms": 800, "srt_passphrase": null }
      },
      "screen": {
        "name": "Pantalla",
        "video": { "source": "Screen", "device": "", "width": 1280, "height": 720, "fps": 30 },
        "audio": { "source": "System", "device": "@DEFAULT_MONITOR@", "bitrate_kbps": 128 },
        "encoder": { "video": "NvencH264", "video_bitrate_kbps": 4000, "keyframe_interval": 60, "quality_preset": "default", "audio": "Opus" },
        "output": { "protocol": "Srt", "uri": "srt://127.0.0.1:9001?mode=caller", "srt_latency_ms": 1200, "srt_passphrase": null }
      },
      "live": {
        "name": "Salida única",
        "video": { "source": "Camera", "device": "", "width": 1280, "height": 720, "fps": 30 },
        "audio": { "source": "Microphone", "device": "default", "bitrate_kbps": 96 },
        "encoder": { "video": "X264", "video_bitrate_kbps": 3000, "keyframe_interval": 60, "quality_preset": "default", "audio": "Opus" },
        "output": { "protocol": "Rtmp", "uri": "rtmp://host/live", "srt_latency_ms": 1200, "srt_passphrase": "secreto" }
      }
    }"#;

    #[test]
    fn migra_el_formato_viejo() {
        let cfg = from_legacy(LEGACY).unwrap();
        assert_eq!(cfg.camera_device, "/dev/video2");
        assert_eq!(cfg.microphone_device, "alsa_input.usb");
        assert_eq!(cfg.video.kind, VideoEncoderKind::X264);
        assert_eq!(cfg.video.bitrate_kbps, 3000);
        assert_eq!(cfg.video.keyframe_interval, 60);
        assert_eq!(cfg.audio.kind, AudioEncoderKind::Opus);
        assert_eq!(cfg.audio.bitrate_kbps, 96);
        assert_eq!(cfg.output.protocol, OutputProtocol::Rtmp);
        assert_eq!(cfg.output.uri, "rtmp://host/live");
        assert_eq!(cfg.output.srt_latency_ms, 1200);
        assert_eq!(cfg.output.srt_passphrase.as_deref(), Some("secreto"));
    }

    #[test]
    fn no_migra_el_formato_nuevo() {
        let cfg = Config::default();
        let texto = serde_json::to_string(&cfg).unwrap();
        assert!(from_legacy(&texto).is_none());
        assert_eq!(serde_json::from_str::<Config>(&texto).unwrap(), cfg);
    }
}
