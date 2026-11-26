use capture_core::{DeviceInfo, DeviceList};
use gstreamer::prelude::*;
use std::collections::HashSet;

/// Extrae la ruta real del dispositivo desde `object.path`.
/// Ejemplo: "v4l2:/dev/video0" -> "/dev/video0".
fn device_path(d: &gstreamer::Device) -> Option<String> {
    // object.path vive en el Structure de propiedades del dispositivo,
    // NO entre las propiedades GObject (find_property no lo encuentra).
    let props = d.properties()?;
    let path = props.get::<String>("object.path").ok()?;
    let path = path.strip_prefix("v4l2:").unwrap_or(&path);
    if path.starts_with("/dev/video") {
        Some(path.to_string())
    } else {
        None
    }
}

/// Nombre real de la fuente de PulseAudio/PipeWire (`node.name`).
/// Es el valor que espera la propiedad `device` de `pulsesrc`.
fn device_source_name(d: &gstreamer::Device) -> Option<String> {
    let props = d.properties()?;
    props.get::<String>("node.name").ok()
}

pub fn enumerate() -> DeviceList {
    let mut list = DeviceList::default();

    // ---------- cámaras ----------
    let monitor = gstreamer::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    let _ = monitor.start();

    let mut seen = HashSet::new();
    for d in monitor.devices() {
        let name = d.display_name().to_string();
        let low = name.to_lowercase();
        if low.contains("screen") || low.contains("ximage") {
            continue;
        }

        let Some(path) = device_path(&d) else { continue; };
        if !path.starts_with("/dev/video") {
            continue;
        }

        // La misma cámara física aparece como /dev/video0 y /dev/video2.
        // Nos quedamos con el primero para no duplicarla en el selector.
        if !seen.insert(name.clone()) {
            continue;
        }

        list.cameras.push(DeviceInfo { id: path, name });
    }
    monitor.stop();

    // ---------- pantalla ----------
    list.screens.push(DeviceInfo {
        id: "0".into(),
        name: "Pantalla completa".into(),
    });

    // ---------- micrófonos ----------
    let monitor = gstreamer::DeviceMonitor::new();
    monitor.add_filter(Some("Audio/Source"), None);
    let _ = monitor.start();

    let mut seen_audio = HashSet::new();
    for d in monitor.devices() {
        let name = d.display_name().to_string();
        let source = device_source_name(&d).unwrap_or_else(|| name.clone());

        // Solo micrófonos: fuera los monitores del sistema.
        if source.ends_with(".monitor") || name.to_lowercase().contains("monitor") {
            continue;
        }
        if !seen_audio.insert(source.clone()) {
            continue;
        }

        list.microphones.push(DeviceInfo { id: source, name });
    }
    monitor.stop();

    list.microphones.insert(
        0,
        DeviceInfo { id: "default".into(), name: "Micrófono por defecto".into() },
    );

    list
}

#[cfg(test)]
mod tests {
    use super::enumerate;

    #[test]
    fn enumera_microfonos() {
        let _ = crate::init();
        let list = enumerate();
        eprintln!("micrófonos: {:?}", list.microphones);
        assert!(list.microphones.iter().any(|m| m.id == "default"));
    }
}
