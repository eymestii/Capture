use crate::device_monitor;
use crate::pipeline::GstPipeline;
use capture_core::{Config, EngineCommand, EngineEvent, PipelineState};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Engine {
    cmd_tx: Sender<EngineCommand>,
    event_rx: Receiver<EngineEvent>,
    handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(config: Config) -> Self {
        let (cmd_tx, cmd_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let handle = thread::spawn(move || run(config, cmd_rx, event_tx));
        Self { cmd_tx, event_rx, handle: Some(handle) }
    }

    pub fn command(&self, cmd: EngineCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn events(&self) -> Vec<EngineEvent> {
        self.event_rx.try_iter().collect()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(EngineCommand::Shutdown);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}

enum BuildResult {
    Done(u64, Result<GstPipeline, String>),
}

/// Construye el pipeline en un hilo aparte: el portal ScreenCast abre un
/// diálogo y puede bloquear. Los builds se serializan: nunca hay dos en vuelo,
/// y un resultado obsoleto se detiene antes de arrancar uno nuevo.
fn start_build(
    config: &Config,
    generation: u64,
    tx: &Sender<EngineEvent>,
    build_tx: &Sender<BuildResult>,
) {
    let _ = tx.send(EngineEvent::State(PipelineState::Starting));

    let cfg = config.clone();
    let build_tx = build_tx.clone();
    thread::spawn(move || {
        let result = match GstPipeline::new_composite(&cfg) {
            Ok(p) => match p.play() {
                Ok(()) => Ok(p),
                Err(e) => {
                    // `play()` puede fallar con el portal ya abierto: cerrarlo
                    // aquí evita dejar una sesión ScreenCast colgada.
                    let _ = p.stop();
                    Err(e.to_string())
                }
            },
            Err(e) => Err(e.to_string()),
        };
        let _ = build_tx.send(BuildResult::Done(generation, result));
    });
}

fn run(mut config: Config, cmd_rx: Receiver<EngineCommand>, tx: Sender<EngineEvent>) {
    let (build_tx, build_rx) = unbounded::<BuildResult>();
    let mut pipeline: Option<GstPipeline> = None;
    let mut building: Option<u64> = None;
    let mut generation: u64 = 0;
    let mut want_running = false;
    let mut pending_rebuild = false;

    let _ = tx.send(EngineEvent::Devices(device_monitor::enumerate()));

    loop {
        let mut shutdown = false;

        for cmd in cmd_rx.try_iter() {
            match cmd {
                EngineCommand::Start => {
                    want_running = true;
                    pending_rebuild = false;
                    if pipeline.is_none() && building.is_none() {
                        generation += 1;
                        building = Some(generation);
                        start_build(&config, generation, &tx, &build_tx);
                    }
                }
                EngineCommand::Stop => {
                    want_running = false;
                    pending_rebuild = false;
                    if let Some(p) = pipeline.take() {
                        let _ = p.stop();
                    }
                    let _ = tx.send(EngineEvent::State(PipelineState::Stopped));
                }
                // Cambios en caliente: no reconstruyen ni cortan el directo.
                EngineCommand::SetVideoBitrate(kbps) => {
                    config.video.bitrate_kbps = kbps;
                    if let Some(p) = pipeline.as_ref() {
                        let _ = p.set_video_bitrate(kbps);
                    }
                }
                EngineCommand::SetKeyframeInterval(frames) => {
                    config.video.keyframe_interval = frames;
                    if let Some(p) = pipeline.as_ref() {
                        let _ = p.set_keyframe_interval(frames);
                    }
                }
                // Cambio estructural: se reconstruye una sola vez, cuando el
                // build en curso (si lo hay) haya terminado.
                EngineCommand::Update(new_config) => {
                    config = new_config;
                    if pipeline.is_some() || building.is_some() {
                        want_running = true;
                        pending_rebuild = true;
                        if building.is_none() {
                            if let Some(p) = pipeline.take() {
                                let _ = p.stop();
                            }
                        }
                    }
                }
                EngineCommand::RefreshDevices => {
                    let _ = tx.send(EngineEvent::Devices(device_monitor::enumerate()));
                }
                EngineCommand::Shutdown => {
                    shutdown = true;
                    break;
                }
            }
        }

        if shutdown {
            if let Some(p) = pipeline.take() {
                let _ = p.stop();
            }
            break;
        }

        // Resultados del build en segundo plano.
        for result in build_rx.try_iter() {
            let BuildResult::Done(gen, result) = result;
            if building != Some(gen) {
                continue;
            }
            building = None;

            match result {
                Ok(p) => {
                    // El build pudo empezar antes de un cambio en caliente:
                    // sincroniza bitrate/keyframes con el estado actual.
                    let _ = p.set_video_bitrate(config.video.bitrate_kbps);
                    let _ = p.set_keyframe_interval(config.video.keyframe_interval);
                    if want_running {
                        if pending_rebuild {
                            // Build viejo: no llega a publicar, se detiene y
                            // el bloque de abajo arranca el nuevo.
                            let _ = p.stop();
                        } else {
                            pipeline = Some(p);
                            let _ = tx.send(EngineEvent::State(PipelineState::Running));
                        }
                    } else {
                        let _ = p.stop();
                        let _ = tx.send(EngineEvent::State(PipelineState::Stopped));
                    }
                }
                Err(e) => {
                    if want_running && !pending_rebuild {
                        eprintln!("[capture] pipeline live falló: {e}");
                        let _ = tx.send(EngineEvent::Error(e));
                        let _ = tx.send(EngineEvent::State(PipelineState::Error));
                        want_running = false;
                    }
                }
            }
        }

        // Rebuild pendiente (un solo build en vuelo a la vez).
        if pending_rebuild && building.is_none() && want_running {
            pending_rebuild = false;
            generation += 1;
            building = Some(generation);
            start_build(&config, generation, &tx, &build_tx);
        }

        // Sondeo del bus para errores y EOS.
        let zero = gstreamer::ClockTime::from_nseconds(0);
        let mut bus_events: Vec<Result<(), String>> = Vec::new();
        if let Some(p) = pipeline.as_ref() {
            if let Some(bus) = p.bus() {
                while let Some(msg) = bus.timed_pop_filtered(
                    zero,
                    &[gstreamer::MessageType::Error, gstreamer::MessageType::Eos],
                ) {
                    match msg.view() {
                        gstreamer::MessageView::Error(err) => {
                            bus_events.push(Err(format!(
                                "{}: {}",
                                err.error(),
                                err.debug().unwrap_or_default()
                            )));
                        }
                        gstreamer::MessageView::Eos(..) => bus_events.push(Ok(())),
                        _ => {}
                    }
                }
            }
        }
        for ev in bus_events {
            match ev {
                Err(text) => {
                    eprintln!("[capture] bus live: {text}");
                    if let Some(p) = pipeline.take() {
                        let _ = p.stop();
                    }
                    want_running = false;
                    pending_rebuild = false;
                    let _ = tx.send(EngineEvent::Error(text));
                    let _ = tx.send(EngineEvent::State(PipelineState::Error));
                }
                Ok(()) => {
                    if let Some(p) = pipeline.take() {
                        let _ = p.stop();
                    }
                    want_running = false;
                    let _ = tx.send(EngineEvent::State(PipelineState::Stopped));
                }
            }
        }

        // Sin pipeline ni build no hay nada que sondear: dormimos más.
        let idle = pipeline.is_none() && building.is_none();
        thread::sleep(if idle {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(20)
        });
    }
}
