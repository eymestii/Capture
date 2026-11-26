use crate::capture::{CameraCapture, MicrophoneCapture, ScreenCapture};
use crate::encoder::{BuiltAudioEncoder, BuiltVideoEncoder};
use crate::output::{RtmpOutput, SrtOutput};
use crate::EngineError;
use capture_core::{Config, OutputProtocol, VideoEncoderKind};
use gstreamer::prelude::*;

const CAMERA_WIDTH: u32 = 1280;
const CAMERA_HEIGHT: u32 = 720;
const SCREEN_WIDTH: u32 = 1280;
const SCREEN_HEIGHT: u32 = 720;
const OUTPUT_WIDTH: u32 = 1280;
const OUTPUT_HEIGHT: u32 = CAMERA_HEIGHT + SCREEN_HEIGHT;
const FPS: u32 = 30;
const AUDIO_RATE: i32 = 48_000;
const AUDIO_CHANNELS: i32 = 2;

// Llamada calificada para evitar la ambigüedad entre ObjectExt::set_name
// (glib) y TypeModuleExt::set_name, ambos traídos por el prelude.
fn set_name(o: &impl IsA<gstreamer::Object>, name: &str) {
    o.set_property("name", name);
}

/// Busca el nodo PipeWire que `pipewiresrc` crea a partir de `pw-dump`.
///
/// `pw-dump` coloca las propiedades dentro de `info.props`, NO en la raíz
/// del objeto. Ese fue el bug que impedía enlazar la pantalla: la función
/// anterior leía `o["props"]`, que nunca existe en nodos/clientes, así que
/// devolvía `None` y el `pw-link` jamás se ejecutaba.
///
/// Nos quedamos solo con `PipeWire:Interface:Node` y comparamos `node.name`
/// (pipewiresrc pone ahí el `client-name`). Así no devolvemos por error el
/// id de un `Client`.
#[cfg(target_os = "linux")]
fn find_pipewire_node(v: &serde_json::Value, name: &str) -> Option<u32> {
    fn props(o: &serde_json::Value) -> Option<&serde_json::Value> {
        o.get("info").and_then(|i| i.get("props"))
    }

    let objects = v.as_array()?;
    let id = |o: &serde_json::Value| o.get("id").and_then(|i| i.as_u64());
    let is_type = |o: &serde_json::Value, t: &str| {
        o.get("type").and_then(|v| v.as_str()) == Some(t)
    };

    // Clientes PipeWire de este proceso: nos permiten preferir el nodo que
    // acaba de crear nuestro pipewiresrc frente a uno viejo de otra app.
    let pid = std::process::id() as u64;
    let own_clients: Vec<u64> = objects
        .iter()
        .filter(|o| is_type(o, "PipeWire:Interface:Client"))
        .filter(|o| {
            props(o)
                .and_then(|p| p.get("application.process.id"))
                .and_then(|v| v.as_u64())
                .map(|p| p == pid)
                .unwrap_or(false)
        })
        .filter_map(id)
        .collect();

    let matching: Vec<(&serde_json::Value, bool)> = objects
        .iter()
        .filter(|o| is_type(o, "PipeWire:Interface:Node"))
        .filter_map(|o| {
            let p = props(o)?;
            if !p.get("node.name")?.as_str()?.contains(name) {
                return None;
            }
            let own = p
                .get("client.id")
                .and_then(|v| v.as_u64())
                .map(|c| own_clients.contains(&c))
                .unwrap_or(false);
            Some((o, own))
        })
        .collect();

    matching
        .iter()
        .find(|(_, own)| *own)
        .or_else(|| matching.first())
        .and_then(|(o, _)| id(o))
        .map(|i| i as u32)
}

/// Enlaza el nodo del portal (salida) con el nodo de `pipewiresrc` (entrada).
/// Reintenta tanto la aparición del nodo como el propio `pw-link` (los puertos
/// pueden tardar un instante en existir). Devuelve el error real si se agota el
/// plazo, para que la UI no se quede en "Iniciando".
#[cfg(target_os = "linux")]
fn link_pipewire_node(
    portal_id: u32,
    client_name: &str,
    timeout: std::time::Duration,
) -> Result<(), EngineError> {
    use std::process::Command;
    use std::time::{Duration, Instant};

    let start = Instant::now();
    let mut attempt: u32 = 0;
    let mut last_error: Option<String> = None;
    let mut logged_nodes = false;

    loop {
        attempt += 1;

        match Command::new("pw-dump").output() {
            Ok(out) if out.status.success() => {
                if let Ok(dump) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    if let Some(src_id) = find_pipewire_node(&dump, client_name) {
                        let link = Command::new("pw-link")
                            .arg(portal_id.to_string())
                            .arg(src_id.to_string())
                            .output()?;
                        if link.status.success() {
                            eprintln!(
                                "[capture] pw-link {portal_id} -> {src_id}: ok (intento {attempt})"
                            );
                            return Ok(());
                        }
                        let err = String::from_utf8_lossy(&link.stderr).trim().to_string();
                        eprintln!(
                            "[capture] pw-link {portal_id} -> {src_id} falló (intento {attempt}): {err}"
                        );
                        last_error = Some(format!("pw-link {portal_id} -> {src_id}: {err}"));
                    } else if !logged_nodes {
                        logged_nodes = true;
                        let nodos: Vec<String> = dump
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter(|o| {
                                        o.get("type").and_then(|t| t.as_str())
                                            == Some("PipeWire:Interface:Node")
                                    })
                                    .filter_map(|o| {
                                        let p = o.get("info")?.get("props")?;
                                        let n = p.get("node.name")?.as_str()?;
                                        let id = o.get("id")?.as_u64()?;
                                        Some(format!("{id}:{n}"))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        eprintln!(
                            "[capture] aún no aparece '{client_name}'. Nodos actuales: {nodos:?}"
                        );
                        last_error = Some(format!("no aparece el nodo '{client_name}'"));
                    }
                }
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if last_error.as_deref() != Some(err.as_str()) {
                    eprintln!("[capture] pw-dump falló: {err}");
                }
                last_error = Some(format!("pw-dump: {err}"));
            }
            Err(e) => {
                last_error = Some(format!("no se pudo ejecutar pw-dump: {e}"));
                eprintln!("{}", last_error.as_deref().unwrap());
            }
        }

        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    Err(format!(
        "timeout tras {attempt} intentos enlazando la pantalla: {}",
        last_error.unwrap_or_else(|| "sin detalles".to_string())
    )
    .into())
}

pub struct GstPipeline {
    pub pipeline: gstreamer::Pipeline,
    /// El encoder que realmente se construyó (puede ser x264 si no hay NVENC),
    /// necesario para aplicar keyframes en caliente con la propiedad correcta.
    video_encoder_kind: VideoEncoderKind,
}

fn add_video_branch(
    pipeline: &gstreamer::Pipeline,
    src: &gstreamer::Element,
    filter: Option<gstreamer::Element>, // capsfilter para excluir DMA-BUF en pantalla
    width: u32,
    height: u32,
    target: &gstreamer::Pad,
) -> Result<(), EngineError> {
    let vconv = gstreamer::ElementFactory::make("videoconvert").build()?;
    let vscale = gstreamer::ElementFactory::make("videoscale").build()?;
    let vrate = gstreamer::ElementFactory::make("videorate").build()?;
    let vcaps = gstreamer::ElementFactory::make("capsfilter").build()?;
    vcaps.set_property(
        "caps",
        &format!(
            "video/x-raw,format=NV12,pixel-aspect-ratio=1/1,width={},height={},framerate={}/1",
            width, height, FPS
        )
        .parse::<gstreamer::Caps>()?,
    );

    pipeline.add(src)?;
    let mut chain: Vec<&gstreamer::Element> = Vec::new();
    if let Some(f) = &filter {
        chain.push(f);
    }
    chain.extend_from_slice(&[&vconv, &vscale, &vrate, &vcaps]);
    for e in chain.iter().copied() {
        pipeline.add(e)?;
    }

    let mut prev = src;
    for e in chain.iter().copied() {
        prev.link(e)?;
        prev = e;
    }
    let out_pad = prev.static_pad("src").ok_or("elemento sin pad src")?;
    out_pad.link(target)?;
    Ok(())
}

impl GstPipeline {
    /// Único pipeline: cámara + pantalla + micrófono -> una salida.
    pub fn new_composite(cfg: &Config) -> Result<Self, EngineError> {
        let pipeline = gstreamer::Pipeline::new();
        set_name(&pipeline, "live");

        // ---- mux + salida (un solo SRT/RTMP) ----
        let (mux, sink) = match cfg.output.protocol {
            OutputProtocol::Srt => SrtOutput {
                uri: cfg.output.uri.clone(),
                latency_ms: cfg.output.srt_latency_ms,
                passphrase: cfg.output.srt_passphrase.clone(),
            }
            .build()?,
            OutputProtocol::Rtmp => RtmpOutput { uri: cfg.output.uri.clone() }.build()?,
        };
        sink.set_property("sync", true);

        let srt_queue = if matches!(cfg.output.protocol, OutputProtocol::Srt) {
            let q = gstreamer::ElementFactory::make("queue").build()?;
            q.set_property("max-size-time", 1_000_000_000u64);
            q.set_property("max-size-buffers", 0u32);
            q.set_property("max-size-bytes", 0u32);
            Some(q)
        } else {
            None
        };

        // ---- compositor: dos entradas, una salida ----
        let comp = gstreamer::ElementFactory::make("compositor")
            .build()
            .or_else(|_| gstreamer::ElementFactory::make("videomixer").build())?;
        set_name(&comp, "mixer");
        pipeline.add(&comp)?;

        let pad_cam = comp
            .request_pad_simple("sink_%u")
            .ok_or("no se pudo crear pad de cámara en el compositor")?;
        let pad_scr = comp
            .request_pad_simple("sink_%u")
            .ok_or("no se pudo crear pad de pantalla en el compositor")?;
        for (pad, y) in [(&pad_cam, 0i32), (&pad_scr, CAMERA_HEIGHT as i32)] {
            pad.set_property("xpos", 0i32);
            pad.set_property("ypos", y);
            pad.set_property("width", OUTPUT_WIDTH as i32);
            pad.set_property("height", CAMERA_HEIGHT as i32);
        }

        // ---- rama cámara ----
        let cam = CameraCapture { device: cfg.camera_device.clone() }.build()?;
        set_name(&cam, "camera-source");
        add_video_branch(&pipeline, &cam, None, CAMERA_WIDTH, CAMERA_HEIGHT, &pad_cam)?;

        // ---- rama pantalla (mantiene el workaround de DMA-BUF) ----
        let scr = ScreenCapture.build()?;
        set_name(&scr, "screen-source");
        let scr_filter = gstreamer::ElementFactory::make("capsfilter").build()?;
        scr_filter.set_property(
            "caps",
            &"video/x-raw,format={ BGRx, BGRA, RGBx, RGBA, NV12, I420, YUY2 }"
                .parse::<gstreamer::Caps>()?,
        );
        add_video_branch(&pipeline, &scr, Some(scr_filter), SCREEN_WIDTH, SCREEN_HEIGHT, &pad_scr)?;

        // ---- salida de video: un solo encoder con el bitrate compartido ----
        let vout_caps = gstreamer::ElementFactory::make("capsfilter").build()?;
        vout_caps.set_property(
            "caps",
            &format!(
                "video/x-raw,format=NV12,pixel-aspect-ratio=1/1,width={},height={},framerate={}/1",
                OUTPUT_WIDTH, OUTPUT_HEIGHT, FPS
            )
            .parse::<gstreamer::Caps>()?,
        );
        let venc = BuiltVideoEncoder::build(
            cfg.video.kind,
            cfg.video.bitrate_kbps,
            cfg.video.keyframe_interval,
        )?;
        set_name(&venc.element, "video-encoder");
        let parse_name = match venc.actual_kind {
            VideoEncoderKind::NvencHevc => "h265parse",
            _ => "h264parse",
        };
        let vparse = gstreamer::ElementFactory::make(parse_name).build()?;
        vparse.set_property("config-interval", -1i32);

        // ---- audio: solo micrófono -> un solo AAC/Opus ----
        let mic = MicrophoneCapture { device: cfg.microphone_device.clone() }.build()?;
        set_name(&mic, "audio-source");
        let aq = gstreamer::ElementFactory::make("queue").build()?;
        let aconv = gstreamer::ElementFactory::make("audioconvert").build()?;
        let ares = gstreamer::ElementFactory::make("audioresample").build()?;
        let acaps = gstreamer::ElementFactory::make("capsfilter").build()?;
        acaps.set_property(
            "caps",
            &gstreamer::Caps::builder("audio/x-raw")
                .field("rate", AUDIO_RATE)
                .field("channels", AUDIO_CHANNELS)
                .build(),
        );

        let aenc = BuiltAudioEncoder::build(cfg.audio.kind, cfg.audio.bitrate_kbps)?;

        // ---- añadir y enlazar ----
        for e in [
            &mic,
            &aq,
            &aconv,
            &ares,
            &acaps,
            &vout_caps,
            &venc.element,
            &vparse,
            &aenc.0,
            &mux,
            &sink,
        ] {
            pipeline.add(e)?;
        }
        if let Some(q) = &srt_queue {
            pipeline.add(q)?;
        }

        gstreamer::Element::link_many(&[&comp, &vout_caps, &venc.element, &vparse, &mux])?;
        gstreamer::Element::link_many(&[&mic, &aq, &aconv, &ares, &acaps, &aenc.0, &mux])?;
        if let Some(q) = &srt_queue {
            mux.link(q)?;
            q.link(&sink)?;
        } else {
            mux.link(&sink)?;
        }

        Ok(Self { pipeline, video_encoder_kind: venc.actual_kind })
    }

    pub fn play(&self) -> Result<(), EngineError> {
        #[cfg(target_os = "linux")]
        {
            let is_pipewire = self.pipeline
                .by_name("screen-source")
                .and_then(|e| e.factory())
                .map(|f| f.name() == "pipewiresrc")
                .unwrap_or(false);
            if is_pipewire {
                let portal_id = crate::capture::PORTAL_NODE_ID
                    .load(std::sync::atomic::Ordering::SeqCst);
                if portal_id == 0 {
                    return Err("el portal ScreenCast no devolvió un nodo PipeWire".into());
                }

                // Con autoconnect=false, pipewiresrc crea su nodo al pasar a
                // PAUSED y se queda esperando a que alguien lo enlace. El
                // problema es que `set_state(PAUSED)` BLOQUEA dentro de
                // pipewiresrc hasta que exista el enlace. Por eso lanzamos en
                // paralelo:
                //   - el cambio a PAUSED (en su propio hilo),
                //   - el `pw-link` que enlaza el nodo del portal con el de
                //     pipewiresrc (en otro hilo).
                // Así el enlace puede ocurrir mientras PAUSED está esperando,
                // sin dejar la UI en "Iniciando" para siempre.
                eprintln!("[capture] enlazando pantalla (portal {portal_id})");

                let (link_tx, link_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = link_tx.send(link_pipewire_node(
                        portal_id,
                        "capture-screen",
                        std::time::Duration::from_secs(20),
                    ));
                });

                let (state_tx, state_rx) = std::sync::mpsc::channel();
                let pipeline_for_state = self.pipeline.clone();
                std::thread::spawn(move || {
                    let _ = state_tx.send(pipeline_for_state.set_state(gstreamer::State::Paused));
                });

                match link_rx.recv_timeout(std::time::Duration::from_secs(25)) {
                    Ok(Ok(())) => {
                        // Enlace hecho: esperamos brevemente a que termine el
                        // cambio a PAUSED, pero si tarda seguimos igual.
                        match state_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                            Ok(Ok(r)) => eprintln!("[capture] set_state(PAUSED) => {r:?}"),
                            Ok(Err(e)) => {
                                eprintln!("[capture] set_state(PAUSED) falló: {e}");
                                let _ = self.pipeline.set_state(gstreamer::State::Null);
                                return Err(format!("no se pudo poner el pipeline en PAUSED: {e}").into());
                            }
                            Err(_) => {
                                eprintln!("[capture] PAUSED aún en curso; continuamos a PLAYING")
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("[capture] enlace de pantalla falló: {e}");
                        let _ = self.pipeline.set_state(gstreamer::State::Null);
                        return Err(e);
                    }
                    Err(_) => {
                        let _ = self.pipeline.set_state(gstreamer::State::Null);
                        return Err("timeout esperando el enlace PipeWire del portal".into());
                    }
                }

                // Verificación opcional: avisa en el log cuando llegue el
                // primer frame real de la pantalla.
                if let Some(pad) = self.pipeline
                    .by_name("screen-source")
                    .and_then(|e| e.static_pad("src"))
                {
                    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let seen_probe = seen.clone();
                    pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, _| {
                        if !seen_probe.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            eprintln!("[capture] primer frame de pantalla recibido");
                        }
                        gstreamer::PadProbeReturn::Ok
                    });
                }
            }
        }
        self.pipeline.set_state(gstreamer::State::Playing)?;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), EngineError> {
        self.pipeline.set_state(gstreamer::State::Null)?;
        // El pipeline siempre incluye la pantalla, así que siempre cerramos
        // la sesión del portal al parar.
        #[cfg(target_os = "linux")]
        crate::capture::close_portal_session();
        Ok(())
    }

    /// Cambia el bitrate sin reconstruir el pipeline.
    pub fn set_video_bitrate(&self, kbps: u32) -> Result<(), EngineError> {
        let enc = self.pipeline.by_name("video-encoder").ok_or("video-encoder no encontrado")?;
        enc.set_property("bitrate", kbps);
        Ok(())
    }

    /// Cambia el intervalo de keyframes sin reconstruir el pipeline.
    pub fn set_keyframe_interval(&self, frames: u32) -> Result<(), EngineError> {
        let enc = self.pipeline.by_name("video-encoder").ok_or("video-encoder no encontrado")?;
        match self.video_encoder_kind {
            VideoEncoderKind::NvencH264 | VideoEncoderKind::NvencHevc => {
                enc.set_property("gop-size", frames as i32);
            }
            VideoEncoderKind::X264 => enc.set_property("key-int-max", frames),
        }
        Ok(())
    }

    pub fn bus(&self) -> Option<gstreamer::Bus> {
        self.pipeline.bus()
    }
}

impl Drop for GstPipeline {
    fn drop(&mut self) {
        // Solo detenemos el pipeline. NO cerramos la sesión del portal aquí:
        // al descartar el resultado tardío de un build viejo (Update/Stop)
        // podríamos cerrar la sesión que ya abrió el pipeline nuevo. El
        // cierre del portal es explícito, en `stop()`.
        let _ = self.pipeline.set_state(gstreamer::State::Null);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::find_pipewire_node;
    use serde_json::json;

    fn dump() -> serde_json::Value {
        // Estructura real de `pw-dump` 1.6: las props viven en `info.props`.
        json!([
            {
                "id": 81,
                "type": "PipeWire:Interface:Client",
                "info": { "props": { "application.name": "gst-launch-1.0" } }
            },
            {
                "id": 82,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "client.id": 81,
                        "media.class": "Stream/Input/Video",
                        "node.name": "capture-screen"
                    }
                }
            },
            {
                "id": 83,
                "type": "PipeWire:Interface:Node",
                "info": { "props": { "node.name": "otro-nodo" } }
            }
        ])
    }

    #[test]
    fn encuentra_el_nodo_de_pipewiresrc() {
        assert_eq!(find_pipewire_node(&dump(), "capture-screen"), Some(82));
    }

    #[test]
    fn no_devuelve_el_id_del_cliente() {
        // El cliente tiene id 81; el nodo correcto es 82.
        assert_ne!(find_pipewire_node(&dump(), "capture-screen"), Some(81));
    }

    #[test]
    fn devuelve_none_si_no_existe() {
        assert_eq!(find_pipewire_node(&dump(), "no-existe"), None);
    }

    #[test]
    fn prefiere_el_nodo_de_nuestro_proceso() {
        let dump = json!([
            {
                "id": 100,
                "type": "PipeWire:Interface:Client",
                "info": {
                    "props": { "application.process.id": std::process::id() }
                }
            },
            {
                "id": 200,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "client.id": 100, "node.name": "capture-screen" }
                }
            },
            {
                "id": 201,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "client.id": 999, "node.name": "capture-screen-viejo" }
                }
            }
        ]);
        assert_eq!(find_pipewire_node(&dump, "capture-screen"), Some(200));
    }
}
