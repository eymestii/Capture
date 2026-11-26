use crate::{factories, EngineError};
use gstreamer::prelude::*;

#[cfg(target_os = "linux")]
pub(crate) static PORTAL_NODE_ID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Mantiene vivos el proxy y la sesión del portal ScreenCast mientras el
/// pipeline de pantalla esté activo. Si se descartan (como pasaba antes),
/// el compositor puede terminar el screencast y `pipewiresrc` deja de
/// recibir frames aunque el pipeline siga en PLAYING.
///
/// Las sesiones se crean y se guardan en un hilo dedicado (`keeper`), de modo
/// que los proxies de ashpd nunca salen de ese hilo y no necesitamos forzar
/// `Send` sobre ellos. Al abrir una sesión nueva se cierra la anterior, y al
/// parar el pipeline se cierra con `close()`.
#[cfg(target_os = "linux")]
mod portal {
    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
    use ashpd::desktop::{PersistMode, Session};
    use ashpd::WindowIdentifier;
    use std::os::fd::OwnedFd;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, OnceLock};

    type Handle = (Screencast<'static>, Session<'static, Screencast<'static>>);

    enum Cmd {
        Open(Sender<Result<(u32, OwnedFd), String>>),
        Close,
    }

    fn keeper(rx: Receiver<Cmd>) {
        let mut current: Option<Handle> = None;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Open(reply) => {
                    if let Some((_, session)) = current.take() {
                        let _ = pollster::block_on(session.close());
                    }
                    match pollster::block_on(open_cast()) {
                        Ok((node, fd, proxy, session)) => {
                            current = Some((proxy, session));
                            let _ = reply.send(Ok((node, fd)));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                }
                Cmd::Close => {
                    if let Some((_, session)) = current.take() {
                        let _ = pollster::block_on(session.close());
                    }
                }
            }
        }
    }

    fn sender() -> &'static Mutex<Sender<Cmd>> {
        static TX: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();
        TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || keeper(rx));
            Mutex::new(tx)
        })
    }

    pub(super) fn open() -> Result<(u32, OwnedFd), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        sender()
            .lock()
            .map_err(|_| "portal: mutex envenenado".to_string())?
            .send(Cmd::Open(reply_tx))
            .map_err(|_| "portal: hilo no disponible".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "portal: el hilo no respondió".to_string())?
    }

    pub(super) fn close() {
        if let Ok(tx) = sender().lock() {
            let _ = tx.send(Cmd::Close);
        }
    }

    async fn open_cast() -> Result<(u32, OwnedFd, Screencast<'static>, Session<'static, Screencast<'static>>), String> {
        let proxy = Screencast::new().await.map_err(|e| e.to_string())?;
        let session = proxy.create_session().await.map_err(|e| e.to_string())?;

        proxy
            .select_sources(
                &session,
                CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                None,
                PersistMode::DoNot,
            )
            .await
            .map_err(|e| e.to_string())?;

        let identifier = WindowIdentifier::default();
        let resp = proxy
            .start(&session, &identifier)
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        let stream = resp.streams().first().ok_or("portal: sin streams")?;
        let node = stream.pipe_wire_node_id();
        let fd = proxy
            .open_pipe_wire_remote(&session)
            .await
            .map_err(|e| e.to_string())?;

        Ok((node, fd, proxy, session))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn close_portal_session() {
    portal::close();
}

pub struct CameraCapture { pub device: String }

impl CameraCapture {
    pub fn build(&self) -> Result<gstreamer::Element, EngineError> {
        let e = gstreamer::ElementFactory::make(factories::camera_element()).build()?;
        if !self.device.is_empty() { e.set_property("device", self.device.as_str()); }
        Ok(e)
    }
}

pub struct ScreenCapture;

impl ScreenCapture {
    #[cfg(target_os = "linux")]
    pub fn build(&self) -> Result<gstreamer::Element, EngineError> {
        use std::os::fd::IntoRawFd;

        let (node_id, fd) = portal::open().map_err(|e| format!("portal ScreenCast: {e}"))?;
        PORTAL_NODE_ID.store(node_id, std::sync::atomic::Ordering::SeqCst);

        let e = gstreamer::ElementFactory::make(factories::screen_element()).build()?;
        // PipeWire 1.6.x no permite autoconectar por id/serial/nombre al nodo
        // del portal ("target not found"). Desactivamos autoconnect y el
        // pipeline enlazará a mano con pw-link tras pasar a PAUSED.
        e.set_property("autoconnect", false);
        e.set_property("client-name", "capture-screen");
        e.set_property("fd", fd.into_raw_fd());
        e.set_property("do-timestamp", true);
        e.set_property("always-copy", true);
        e.set_property("keepalive-time", 100i32);
        Ok(e)
    }

    #[cfg(not(target_os = "linux"))]
    pub fn build(&self) -> Result<gstreamer::Element, EngineError> {
        let e = gstreamer::ElementFactory::make(factories::screen_element()).build()?;
        #[cfg(target_os = "macos")]
        {
            e.set_property("capture-screen", true);
        }
        Ok(e)
    }
}

pub struct MicrophoneCapture { pub device: String }

impl MicrophoneCapture {
    pub fn build(&self) -> Result<gstreamer::Element, EngineError> {
        let e = gstreamer::ElementFactory::make(factories::microphone_element()).build()?;
        if !self.device.is_empty() && self.device != "default" {
            e.set_property("device", self.device.as_str());
        }
        Ok(e)
    }
}
