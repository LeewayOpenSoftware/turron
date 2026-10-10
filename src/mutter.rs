//! Sesion directa contra la API privada de Mutter (org.gnome.Mutter.RemoteDesktop /
//! org.gnome.Mutter.ScreenCast), la misma que usa gnome-remote-desktop. No muestra dialogos.
//!
//! Orden obligatorio: CreateSession de RemoteDesktop -> CreateSession de ScreenCast enlazada
//! (`remote-desktop-session-id`) -> RecordMonitor/RecordVirtual -> Start de RemoteDesktop.
//! La senal `PipeWireStreamAdded` entrega el nodo de PipeWire y hay que escucharla ANTES del Start.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::oneshot;
use zbus::{zvariant::{OwnedObjectPath, OwnedValue, Value}, Connection, Proxy};

const RD_DEST: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const RD_IFACE: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_SESSION_IFACE: &str = "org.gnome.Mutter.RemoteDesktop.Session";
const SC_DEST: &str = "org.gnome.Mutter.ScreenCast";
const SC_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SC_IFACE: &str = "org.gnome.Mutter.ScreenCast";
const SC_SESSION_IFACE: &str = "org.gnome.Mutter.ScreenCast.Session";
const SC_STREAM_IFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";
const STREAM_WAIT: Duration = Duration::from_secs(8);

/// Un stream de monitor ya publicado en PipeWire.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub node_id: u32,
    pub path: OwnedObjectPath,
    pub position: (i32, i32),
    pub size: (i32, i32),
}

pub struct MutterSession {
    rd: Proxy<'static>,
    sc_session: Proxy<'static>,
    conn: Connection,
    stream_path: Option<OwnedObjectPath>,
    node_rx: Option<oneshot::Receiver<u32>>,
    pub stream: Option<StreamInfo>,
    started: bool,
}

/// Opciones del portal (`cursor_mode`: 1 oculto, 2 incrustado, 4 metadata) -> Mutter (0, 1, 2).
pub fn mutter_cursor_mode(portal_mode: Option<u32>) -> u32 {
    match portal_mode {
        Some(2) => 1,
        Some(4) => 2,
        _ => 0,
    }
}

impl MutterSession {
    pub async fn create(conn: &Connection) -> Result<MutterSession> {
        let rd_root = Proxy::new(conn, RD_DEST, RD_PATH, RD_IFACE).await?;
        let rd_path: OwnedObjectPath = rd_root.call("CreateSession", &()).await.context("RemoteDesktop.CreateSession")?;
        let rd = Proxy::new(conn, RD_DEST, rd_path.clone(), RD_SESSION_IFACE).await?;
        let session_id: String = rd.get_property("SessionId").await.context("SessionId")?;

        let sc_root = Proxy::new(conn, SC_DEST, SC_PATH, SC_IFACE).await?;
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("remote-desktop-session-id", Value::from(session_id));
        let sc_path: OwnedObjectPath = sc_root.call("CreateSession", &(props,)).await.context("ScreenCast.CreateSession")?;
        let sc_session = Proxy::new(conn, SC_DEST, sc_path, SC_SESSION_IFACE).await?;
        Ok(MutterSession { rd, sc_session, conn: conn.clone(), stream_path: None, node_rx: None, stream: None, started: false })
    }

    /// Pide el stream del monitor `connector` ("" = monitor primario en la API de Mutter). Si no hay monitor
    /// (o falla) usa un monitor virtual.
    pub async fn record_monitor(&mut self, cursor_mode: u32, connector: &str) -> Result<()> {
        if self.stream_path.is_some() {
            return Ok(());
        }
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("cursor-mode", Value::from(cursor_mode));
        let path: OwnedObjectPath = match self.sc_session.call("RecordMonitor", &(connector, props.clone())).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("RecordMonitor fallo ({e}); se prueba RecordVirtual");
                let mut vprops = props;
                vprops.insert("is-platform", Value::from(false));
                self.sc_session.call("RecordVirtual", &(vprops,)).await.context("RecordVirtual")?
            }
        };

        // La senal se escucha antes del Start para no perderla.
        let stream = Proxy::new(&self.conn, SC_DEST, path.clone(), SC_STREAM_IFACE).await?;
        let mut signals = stream.receive_signal("PipeWireStreamAdded").await?;
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            if let Some(msg) = signals.next().await {
                if let Ok(node) = msg.body().deserialize::<u32>() {
                    let _ = tx.send(node);
                }
            }
        });
        self.stream_path = Some(path);
        self.node_rx = Some(rx);
        Ok(())
    }

    /// Arranca la sesion y espera el nodo de PipeWire. Devuelve los streams (0 o 1).
    pub async fn start(&mut self) -> Result<Vec<StreamInfo>> {
        if !self.started {
            self.rd.call::<_, _, ()>("Start", &()).await.context("RemoteDesktop.Session.Start")?;
            self.started = true;
        }
        if self.stream.is_none() {
            if let (Some(rx), Some(path)) = (self.node_rx.take(), self.stream_path.clone()) {
                let node_id = tokio::time::timeout(STREAM_WAIT, rx)
                    .await
                    .map_err(|_| anyhow!("Mutter no publico el stream de PipeWire en {STREAM_WAIT:?}"))?
                    .map_err(|_| anyhow!("se cerro la senal PipeWireStreamAdded"))?;
                let (position, size) = self.geometry(&path).await;
                self.stream = Some(StreamInfo { node_id, path, position, size });
            }
        }
        Ok(self.stream.iter().cloned().collect())
    }

    async fn geometry(&self, path: &OwnedObjectPath) -> ((i32, i32), (i32, i32)) {
        let mut out = ((0, 0), (0, 0));
        let Ok(stream) = Proxy::new(&self.conn, SC_DEST, path.clone(), SC_STREAM_IFACE).await else { return out };
        let Ok(params) = stream.get_property::<HashMap<String, OwnedValue>>("Parameters").await else { return out };
        let pair = |key: &str| -> Option<(i32, i32)> { <(i32, i32)>::try_from(params.get(key)?.try_clone().ok()?).ok() };
        if let Some(p) = pair("position") {
            out.0 = p;
        }
        if let Some(s) = pair("size") {
            out.1 = s;
        }
        out
    }

    pub async fn stop(&self) {
        if let Err(e) = self.rd.call::<_, _, ()>("Stop", &()).await {
            tracing::debug!("Session.Stop: {e}");
        }
    }

    // ---- Input (RemoteDesktop.Session.Notify*) -------------------------------------------

    pub async fn pointer_motion(&self, dx: f64, dy: f64) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyPointerMotionRelative", &(dx, dy)).await?)
    }

    pub async fn pointer_motion_absolute(&self, node_id: u32, x: f64, y: f64) -> Result<()> {
        let path = self.stream_for(node_id)?;
        Ok(self.rd.call::<_, _, ()>("NotifyPointerMotionAbsolute", &(path.as_str(), x, y)).await?)
    }

    pub async fn pointer_button(&self, button: i32, pressed: bool) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyPointerButton", &(button, pressed)).await?)
    }

    pub async fn pointer_axis(&self, dx: f64, dy: f64) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyPointerAxis", &(dx, dy, 0u32)).await?)
    }

    pub async fn pointer_axis_discrete(&self, axis: u32, steps: i32) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyPointerAxisDiscrete", &(axis, steps)).await?)
    }

    pub async fn keyboard_keycode(&self, keycode: i32, pressed: bool) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyKeyboardKeycode", &(keycode as u32, pressed)).await?)
    }

    pub async fn keyboard_keysym(&self, keysym: i32, pressed: bool) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyKeyboardKeysym", &(keysym as u32, pressed)).await?)
    }

    pub async fn touch_down(&self, node_id: u32, slot: u32, x: f64, y: f64) -> Result<()> {
        let path = self.stream_for(node_id)?;
        Ok(self.rd.call::<_, _, ()>("NotifyTouchDown", &(path.as_str(), slot, x, y)).await?)
    }

    pub async fn touch_motion(&self, node_id: u32, slot: u32, x: f64, y: f64) -> Result<()> {
        let path = self.stream_for(node_id)?;
        Ok(self.rd.call::<_, _, ()>("NotifyTouchMotion", &(path.as_str(), slot, x, y)).await?)
    }

    pub async fn touch_up(&self, slot: u32) -> Result<()> {
        Ok(self.rd.call::<_, _, ()>("NotifyTouchUp", &(slot,)).await?)
    }

    fn stream_for(&self, node_id: u32) -> Result<String> {
        match &self.stream {
            Some(s) if s.node_id == node_id => Ok(s.path.to_string()),
            Some(s) => bail!("stream {node_id} desconocido (el activo es {})", s.node_id),
            None => bail!("la sesion todavia no tiene stream"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_modes_map_to_mutter() {
        assert_eq!(mutter_cursor_mode(None), 0);
        assert_eq!(mutter_cursor_mode(Some(1)), 0);
        assert_eq!(mutter_cursor_mode(Some(2)), 1);
        assert_eq!(mutter_cursor_mode(Some(4)), 2);
    }
}
