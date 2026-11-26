use serde::{Deserialize, Serialize};

// ---------- modelos ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoEncoderKind { NvencH264, NvencHevc, X264 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioEncoderKind { Opus, Aac }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputProtocol { Srt, Rtmp }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoEncoding {
    pub kind: VideoEncoderKind,
    pub bitrate_kbps: u32,
    pub keyframe_interval: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioEncoding {
    pub kind: AudioEncoderKind,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputSettings {
    pub protocol: OutputProtocol,
    pub uri: String,
    pub srt_latency_ms: u32,
    pub srt_passphrase: Option<String>,
}

/// Lo que el pipeline usa de verdad: una cámara, un micrófono y una única salida.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub camera_device: String,
    pub microphone_device: String,
    pub video: VideoEncoding,
    pub audio: AudioEncoding,
    pub output: OutputSettings,
}

// ---------- dispositivos ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceList {
    pub cameras: Vec<DeviceInfo>,
    pub screens: Vec<DeviceInfo>,
    pub microphones: Vec<DeviceInfo>,
}

// ---------- estado / mensajería ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState { Stopped, Starting, Running, Error }

#[derive(Debug, Clone)]
pub enum EngineCommand {
    Start,
    Stop,
    /// Se aplica en caliente, sin reconstruir el pipeline.
    SetVideoBitrate(u32),
    /// Se aplica en caliente, sin reconstruir el pipeline.
    SetKeyframeInterval(u32),
    /// Cambio estructural: reconstruye el pipeline.
    Update(Config),
    RefreshDevices,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    State(PipelineState),
    Error(String),
    Devices(DeviceList),
}

// ---------- defaults ----------

impl Default for VideoEncoding {
    fn default() -> Self {
        Self { kind: VideoEncoderKind::NvencH264, bitrate_kbps: 2400, keyframe_interval: 60 }
    }
}

impl Default for AudioEncoding {
    fn default() -> Self {
        Self { kind: AudioEncoderKind::Aac, bitrate_kbps: 96 }
    }
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            protocol: OutputProtocol::Srt,
            uri: "srt://109.123.252.126:9000?mode=caller&streamid=default/app/live".into(),
            srt_latency_ms: 1200,
            srt_passphrase: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            camera_device: String::new(),
            microphone_device: "default".into(),
            video: VideoEncoding::default(),
            audio: AudioEncoding::default(),
            output: OutputSettings::default(),
        }
    }
}
