use crate::EngineError;
use gstreamer::prelude::*;

pub struct SrtOutput {
    pub uri: String,
    pub latency_ms: u32,
    pub passphrase: Option<String>,
}

impl SrtOutput {
    fn full_uri(&self) -> String {
        let mut uri = self.uri.clone();
        let sep = if uri.contains('?') { '&' } else { '?' };
        // El URI de srtsink espera `latency` en MILISEGUNDOS (confirmado con
        // gst-inspect y probando el parseo del URI con GStreamer).
        if !uri.contains("latency=") {
            uri.push_str(&format!("{}latency={}", sep, self.latency_ms));
        }
        if let Some(pass) = &self.passphrase {
            if !pass.is_empty() && !uri.contains("passphrase=") {
                uri.push_str(&format!("&passphrase={}", pass));
            }
        }
        uri
    }

    pub fn build(&self) -> Result<(gstreamer::Element, gstreamer::Element), EngineError> {
        let mux = gstreamer::ElementFactory::make("mpegtsmux").build()?;
        // 2 s de colchón en el mux. Con 300 ms, OME recibe el PMT antes de que
        // lleguen los primeros frames de la cámara y crea el stream solo con
        mux.set_property("latency", 500_000_000u64);
        mux.set_property("bitrate", 2_400_000u64);
        let sink = gstreamer::ElementFactory::make("srtsink").build()?;
        sink.set_property("uri", self.full_uri().as_str());
        sink.set_property("blocksize", 1316u32);
        Ok((mux, sink))
    }
}

pub struct RtmpOutput { pub uri: String }

impl RtmpOutput {
    pub fn build(&self) -> Result<(gstreamer::Element, gstreamer::Element), EngineError> {
        let mux = gstreamer::ElementFactory::make("flvmux").build()?;
        mux.set_property("streamable", true);
        let sink = gstreamer::ElementFactory::make("rtmpsink").build()?;
        sink.set_property("location", self.uri.as_str());
        Ok((mux, sink))
    }
}
