mod config;
mod ui;

use capture_core::{Config, DeviceList, EngineCommand, EngineEvent, PipelineState};
use capture_engine::Engine;
use eframe::egui;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    if let Err(e) = capture_engine::init() {
        eprintln!("No se pudo inicializar GStreamer: {e}");
        std::process::exit(1);
    }
    let cfg = config::load();
    let app = App::new(cfg);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 860.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Capture · cámara + pantalla",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    )
}

pub struct App {
    pub config: Config,
    pub engine: Engine,
    pub devices: DeviceList,
    pub state: PipelineState,
    pub error: Option<String>,
    pub log: Vec<String>,
    /// Persistencia y reconstrucción con retardo: arrastrar un slider o
    /// escribir una URI no debe escribir el JSON ni reiniciar el directo
    /// en cada frame.
    pub pending_save: bool,
    pub pending_apply: bool,
    pub last_change: Instant,
}

impl App {
    fn new(config: Config) -> Self {
        Self {
            engine: Engine::start(config.clone()),
            config,
            devices: DeviceList::default(),
            state: PipelineState::Stopped,
            error: None,
            log: Vec::new(),
            pending_save: false,
            pending_apply: false,
            last_change: Instant::now(),
        }
    }

    fn drain_events(&mut self) {
        for ev in self.engine.events() {
            match ev {
                EngineEvent::State(s) => {
                    self.log.push(format!("{s:?}"));
                    self.state = s;
                    if s != PipelineState::Error {
                        self.error = None;
                    }
                }
                EngineEvent::Error(e) => {
                    self.log.push(format!("ERROR {e}"));
                    self.error = Some(e);
                }
                EngineEvent::Devices(d) => {
                    self.log.push("Dispositivos actualizados".into());
                    self.devices = d;

                    // Si el micrófono guardado ya no existe, volvemos al
                    // micrófono por defecto.
                    let current = self.config.microphone_device.clone();
                    let known = current == "default"
                        || self.devices.microphones.iter().any(|m| m.id == current);
                    if !known {
                        self.config.microphone_device = "default".into();
                        config::save(&self.config);
                        self.engine.command(EngineCommand::Update(self.config.clone()));
                    }
                }
            }
        }
        if self.log.len() > 200 {
            let extra = self.log.len() - 200;
            self.log.drain(..extra);
        }
    }

    fn flush_changes(&mut self) {
        if !(self.pending_save || self.pending_apply) {
            return;
        }
        if self.last_change.elapsed() < Duration::from_millis(500) {
            return;
        }
        if self.pending_save {
            config::save(&self.config);
            self.pending_save = false;
        }
        if self.pending_apply {
            self.engine.command(EngineCommand::Update(self.config.clone()));
            self.pending_apply = false;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.flush_changes();
        ui::draw(self, ctx);
        // Sin preview: no hace falta repintar a 30 fps. Solo cuando hay cambio.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl Drop for App {
    /// No perder el último cambio pendiente (eframe no garantiza `save`,
    /// porque la persistencia propia está desactivada).
    fn drop(&mut self) {
        config::save(&self.config);
    }
}
