use crate::App;
use capture_core::*;
use eframe::egui;
use std::time::Instant;

pub fn draw(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("log")
        .max_height(150.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for line in &app.log { ui.monospace(line); }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
        card(ui, app);
    });
});
}

fn card(ui: &mut egui::Ui, app: &mut App) {
    ui.push_id("live", |ui| {
        let mut p = app.config.clone();
        let before = p.clone();

        // ---- estado ----
        ui.horizontal(|ui| {
            match app.state {
                PipelineState::Running => {
                    ui.colored_label(egui::Color32::GREEN, "En vivo");
                    if ui.button("⏹ Detener").clicked() {
                        app.engine.command(EngineCommand::Stop);
                    }
                }
                PipelineState::Error => {
                    ui.colored_label(egui::Color32::RED, "Error");
                    if ui.button("Reintentar").clicked() {
                        start(app);
                    }
                }
                PipelineState::Starting => {
                    ui.colored_label(egui::Color32::YELLOW, "Iniciando");
                    ui.add_enabled(false, egui::Button::new("Iniciar"));
                }
                PipelineState::Stopped => {
                    ui.colored_label(egui::Color32::GRAY, "Detenido");
                    if ui.button("Iniciar").clicked() {
                        start(app);
                    }
                }
            }
        });
        if let Some(e) = &app.error {
            ui.colored_label(egui::Color32::RED, e);
        }
        ui.separator();

        // ---- fuentes: solo se eligen dispositivos ----
        let cams = app.devices.cameras.clone();
        let mics = app.devices.microphones.clone();
        let screens = app.devices.screens.clone();

        egui::ComboBox::from_label("Cámara")
            .selected_text(
                cams.iter()
                    .find(|d| d.id == p.camera_device)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| p.camera_device.clone()),
            )
            .show_ui(ui, |ui| {
                for d in &cams {
                    ui.selectable_value(&mut p.camera_device, d.id.clone(), &d.name);
                }
            });

        egui::ComboBox::from_label("Micrófono")
            .selected_text(
                mics.iter()
                    .find(|d| d.id == p.microphone_device)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| p.microphone_device.clone()),
            )
            .show_ui(ui, |ui| {
                for d in &mics {
                    ui.selectable_value(&mut p.microphone_device, d.id.clone(), &d.name);
                }
            });

        ui.separator();

        // ---- codificación ----
        egui::ComboBox::from_label("Encoder video")
            .selected_text(format!("{:?}", p.video.kind))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.video.kind, VideoEncoderKind::NvencH264, "NVENC H.264");
                ui.selectable_value(&mut p.video.kind, VideoEncoderKind::NvencHevc, "NVENC H.265");
                ui.selectable_value(&mut p.video.kind, VideoEncoderKind::X264, "x264 (software)");
            });

        // Bitrate y keyframes se aplican en caliente: no cortan el directo.
        let bitrate = ui.add(
            egui::Slider::new(&mut p.video.bitrate_kbps, 500..=20000)
                .text("Bitrate video"),
        );
        if bitrate.changed() {
            app.engine.command(EngineCommand::SetVideoBitrate(p.video.bitrate_kbps));
        }

        let keyframe = ui.add(
            egui::Slider::new(&mut p.video.keyframe_interval, 15..=250)
                .text("Keyframe interval"),
        );
        if keyframe.changed() {
            app.engine.command(EngineCommand::SetKeyframeInterval(p.video.keyframe_interval));
        }

        egui::ComboBox::from_label("Encoder audio")
            .selected_text(format!("{:?}", p.audio.kind))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.audio.kind, AudioEncoderKind::Aac, "AAC (recomendado)");
                ui.selectable_value(&mut p.audio.kind, AudioEncoderKind::Opus, "Opus");
            });
        ui.add(egui::Slider::new(&mut p.audio.bitrate_kbps, 32..=320).text("Bitrate audio"));

        ui.separator();

        // ---- salida única ----
        egui::ComboBox::from_label("Protocolo")
            .selected_text(format!("{:?}", p.output.protocol))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.output.protocol, OutputProtocol::Srt, "SRT");
                ui.selectable_value(&mut p.output.protocol, OutputProtocol::Rtmp, "RTMP");
            });
        ui.label("Destino");
        ui.text_edit_singleline(&mut p.output.uri);
        if p.output.protocol == OutputProtocol::Srt {
            ui.add(
                egui::Slider::new(&mut p.output.srt_latency_ms, 20..=2000)
                    .text("Latencia SRT"),
            );
            ui.label("Passphrase");
            let mut pass = p.output.srt_passphrase.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut pass).changed() {
                p.output.srt_passphrase = if pass.is_empty() { None } else { Some(pass) };
            }
        }

        // ---- propagar cambios ----
        if p != before {
            app.config = p.clone();
            app.pending_save = true;
            app.last_change = Instant::now();
        }

        // Bitrate y keyframes van por comandos en caliente; cualquier otro
        // cambio es estructural y se aplica con retardo (500 ms) para no
        // reconstruir el pipeline en cada tecla o frame de arrastre.
        let mut normalized = p;
        normalized.video.bitrate_kbps = before.video.bitrate_kbps;
        normalized.video.keyframe_interval = before.video.keyframe_interval;
        if normalized != before {
            app.pending_apply = true;
        }
    });
}

fn start(app: &mut App) {
    // El motor arranca con la última configuración, no con la del inicio.
    app.pending_apply = false;
    app.engine.command(EngineCommand::Update(app.config.clone()));
    app.engine.command(EngineCommand::Start);
}
