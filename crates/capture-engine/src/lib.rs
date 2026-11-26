mod capture;
mod device_monitor;
mod encoder;
mod engine;
mod factories;
mod output;
mod pipeline;

pub use engine::Engine;

// `gstreamer::Error` y `gstreamer::glib::Error` son el mismo tipo en esta
// versión, así que dos `#[from]` chocaban. Un alias evita el conflicto y
// hace que todos los `?` funcionen directamente.
pub type EngineError = Box<dyn std::error::Error + Send + Sync>;

pub fn init() -> Result<(), gstreamer::glib::Error> {
    gstreamer::init()
}
