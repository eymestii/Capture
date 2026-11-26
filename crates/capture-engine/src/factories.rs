use capture_core::{AudioEncoderKind, VideoEncoderKind};

#[cfg(target_os = "linux")]
pub fn camera_element() -> &'static str { "v4l2src" }
#[cfg(target_os = "windows")]
pub fn camera_element() -> &'static str { "dshowvideosrc" }
#[cfg(target_os = "macos")]
pub fn camera_element() -> &'static str { "avfvideosrc" }

#[cfg(target_os = "linux")]
pub fn screen_element() -> &'static str { "pipewiresrc" }
#[cfg(target_os = "windows")]
pub fn screen_element() -> &'static str { "gdiscreen" }
#[cfg(target_os = "macos")]
// Requiere la opción `capture-screen` (se aplica en `ScreenCapture::build`).
pub fn screen_element() -> &'static str { "avfvideosrc" }

#[cfg(target_os = "linux")]
pub fn microphone_element() -> &'static str { "pulsesrc" }
#[cfg(target_os = "windows")]
pub fn microphone_element() -> &'static str { "wasapi2src" }
#[cfg(target_os = "macos")]
pub fn microphone_element() -> &'static str { "avfaudiosrc" }

pub fn video_encoder(kind: VideoEncoderKind) -> &'static str {
    match kind {
        VideoEncoderKind::NvencH264 => "nvh264enc",
        VideoEncoderKind::NvencHevc => "nvh265enc",
        VideoEncoderKind::X264 => "x264enc",
    }
}

pub fn audio_encoder(kind: AudioEncoderKind) -> (&'static str, Option<&'static str>) {
    match kind {
        AudioEncoderKind::Opus => ("opusenc", None),
        AudioEncoderKind::Aac => ("faac", Some("avenc_aac")),
    }
}
