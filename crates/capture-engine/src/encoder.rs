use crate::{factories, EngineError};
use capture_core::{AudioEncoderKind, VideoEncoderKind};
use gstreamer::prelude::*;

pub struct BuiltVideoEncoder {
    pub element: gstreamer::Element,
    pub actual_kind: VideoEncoderKind,
}

impl BuiltVideoEncoder {
    pub fn build(kind: VideoEncoderKind, bitrate_kbps: u32, keyframe: u32) -> Result<Self, EngineError> {
        let name = factories::video_encoder(kind);
        let (element, actual_kind) = match gstreamer::ElementFactory::make(name).build() {
            Ok(e) => (e, kind),
            Err(_) => (gstreamer::ElementFactory::make("x264enc").build()?, VideoEncoderKind::X264),
        };
        element.set_property("bitrate", bitrate_kbps);
        match actual_kind {
            VideoEncoderKind::NvencH264 | VideoEncoderKind::NvencHevc => {
                element.set_property("gop-size", keyframe as i32);
                element.set_property("repeat-sequence-header", true);
                element.set_property_from_str("rc-mode", "cbr");
                element.set_property("zerolatency", true);
                element.set_property("vbv-buffer-size", bitrate_kbps);
                element.set_property("bframes", 0u32);
            }
            VideoEncoderKind::X264 => element.set_property("key-int-max", keyframe),
        }
        Ok(Self { element, actual_kind })
    }
}

pub struct BuiltAudioEncoder(pub gstreamer::Element);

impl BuiltAudioEncoder {
    pub fn build(kind: AudioEncoderKind, bitrate_kbps: u32) -> Result<Self, EngineError> {
        let (primary, fallback) = factories::audio_encoder(kind);
        let e = match gstreamer::ElementFactory::make(primary).build() {
            Ok(e) => e,
            Err(_) => {
                let fallback = fallback.ok_or_else(|| format!("no hay encoder de audio para {kind:?}"))?;
                gstreamer::ElementFactory::make(fallback).build()?
            }
        };
        e.set_property("bitrate", (bitrate_kbps * 1000) as i32);
        Ok(Self(e))
    }
}
