//! org.freedesktop.impl.portal.RemoteDesktop

use crate::forward::{self, PortalResult, PORTAL_PATH, REMOTEDESKTOP_IFACE};
use crate::mutter::mutter_cursor_mode;
use crate::session::{create_if_allowed, opt_u32, register_forward, start_results, Created};
use crate::state::{Kind, State};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::{fdo, interface, zvariant::{ObjectPath, OwnedValue, Value}};

pub struct RemoteDesktop {
    state: Arc<State>,
}

impl RemoteDesktop {
    pub fn new(state: Arc<State>) -> Self {
        RemoteDesktop { state }
    }
}

type Options = HashMap<String, OwnedValue>;

const RESPONSE_OK: u32 = 0;
const RESPONSE_OTHER: u32 = 2;

fn input_error(e: anyhow::Error) -> fdo::Error {
    fdo::Error::Failed(format!("{e:#}"))
}

/// Notificacion de input: a Mutter si la sesion es propia, al backend de GNOME si es reenviada.
macro_rules! notify {
    ($self:ident, $session:ident, $options:ident, $method:literal, ($($arg:expr),*), $mutter:ident => $call:expr) => {{
        let st = &$self.state;
        match st.get($session.as_str()).await {
            None => Err(fdo::Error::Failed("sesion desconocida".into())),
            Some(entry) => match &entry.kind {
                Kind::Forward => forward::call_unit(&st.conn, PORTAL_PATH, REMOTEDESKTOP_IFACE, $method, &($session.clone(), $options $(, $arg)*)).await,
                Kind::Mutter(m) => {
                    let $mutter = m.lock().await;
                    $call.await.map_err(input_error)
                }
            },
        }
    }};
}

#[interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktop {
    #[zbus(property)]
    async fn available_device_types(&self) -> u32 {
        7
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    async fn create_session(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, options: Options) -> fdo::Result<PortalResult> {
        let (st, sh) = (&self.state, session_handle.as_str());
        let caller = match create_if_allowed(st, handle.as_str(), sh).await? {
            Created::Approved => return Ok((RESPONSE_OK, HashMap::new())),
            Created::Forward(c) => c,
        };
        let r = forward::call_result(&st.conn, PORTAL_PATH, REMOTEDESKTOP_IFACE, "CreateSession", &(handle, session_handle.clone(), app_id, options)).await?;
        if r.0 == RESPONSE_OK {
            register_forward(st, sh, caller).await?;
        }
        Ok(r)
    }

    async fn select_devices(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, options: Options) -> fdo::Result<PortalResult> {
        let st = &self.state;
        let Some(entry) = st.get(session_handle.as_str()).await else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        match &entry.kind {
            Kind::Forward => forward::call_result(&st.conn, PORTAL_PATH, REMOTEDESKTOP_IFACE, "SelectDevices", &(handle, session_handle, app_id, options)).await,
            Kind::Mutter(_) => {
                if let Some(t) = opt_u32(&options, "types") {
                    *entry.device_types.lock().await = t;
                }
                if let Some(p) = opt_u32(&options, "persist_mode") {
                    *entry.persist_mode.lock().await = p;
                }
                Ok((RESPONSE_OK, HashMap::new()))
            }
        }
    }

    async fn start(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, parent_window: &str, options: Options) -> fdo::Result<PortalResult> {
        let st = &self.state;
        let Some(entry) = st.get(session_handle.as_str()).await else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        match &entry.kind {
            Kind::Forward => forward::call_result(&st.conn, PORTAL_PATH, REMOTEDESKTOP_IFACE, "Start", &(handle, session_handle, app_id, parent_window, options)).await,
            Kind::Mutter(m) => {
                let mut m = m.lock().await;
                // Sesion de solo input (nadie llamo a SelectSources): igual hace falta un stream
                // para el input absoluto, asi que se pide el monitor primario.
                let cursor = *entry.cursor_mode.lock().await;
                if let Err(e) = m.record_monitor(mutter_cursor_mode(cursor), entry.monitor.as_deref().unwrap_or("")).await {
                    tracing::warn!("RemoteDesktop.Start: sin stream ({e:#})");
                }
                match m.start().await {
                    Ok(streams) => {
                        let persist = *entry.persist_mode.lock().await;
                        let mut results = start_results(&streams, persist);
                        let devices = *entry.device_types.lock().await;
                        for (k, v) in [("devices", Value::from(devices)), ("clipboard_enabled", Value::from(false))] {
                            if let Ok(v) = OwnedValue::try_from(v) {
                                results.insert(k.to_owned(), v);
                            }
                        }
                        tracing::info!("RemoteDesktop.Start sin dialogo: {} stream(s)", streams.len());
                        Ok((RESPONSE_OK, results))
                    }
                    Err(e) => {
                        tracing::error!("RemoteDesktop.Start: {e:#}");
                        Ok((RESPONSE_OTHER, HashMap::new()))
                    }
                }
            }
        }
    }

    async fn notify_pointer_motion(&self, session_handle: ObjectPath<'_>, options: Options, dx: f64, dy: f64) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyPointerMotion", (dx, dy), m => m.pointer_motion(dx, dy))
    }

    async fn notify_pointer_motion_absolute(&self, session_handle: ObjectPath<'_>, options: Options, stream: u32, x: f64, y: f64) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyPointerMotionAbsolute", (stream, x, y), m => m.pointer_motion_absolute(stream, x, y))
    }

    async fn notify_pointer_button(&self, session_handle: ObjectPath<'_>, options: Options, button: i32, state: u32) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyPointerButton", (button, state), m => m.pointer_button(button, state != 0))
    }

    async fn notify_pointer_axis(&self, session_handle: ObjectPath<'_>, options: Options, dx: f64, dy: f64) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyPointerAxis", (dx, dy), m => m.pointer_axis(dx, dy))
    }

    async fn notify_pointer_axis_discrete(&self, session_handle: ObjectPath<'_>, options: Options, axis: u32, steps: i32) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyPointerAxisDiscrete", (axis, steps), m => m.pointer_axis_discrete(axis, steps))
    }

    async fn notify_keyboard_keycode(&self, session_handle: ObjectPath<'_>, options: Options, keycode: i32, state: u32) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyKeyboardKeycode", (keycode, state), m => m.keyboard_keycode(keycode, state != 0))
    }

    async fn notify_keyboard_keysym(&self, session_handle: ObjectPath<'_>, options: Options, keysym: i32, state: u32) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyKeyboardKeysym", (keysym, state), m => m.keyboard_keysym(keysym, state != 0))
    }

    async fn notify_touch_down(&self, session_handle: ObjectPath<'_>, options: Options, stream: u32, slot: u32, x: f64, y: f64) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyTouchDown", (stream, slot, x, y), m => m.touch_down(stream, slot, x, y))
    }

    async fn notify_touch_motion(&self, session_handle: ObjectPath<'_>, options: Options, stream: u32, slot: u32, x: f64, y: f64) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyTouchMotion", (stream, slot, x, y), m => m.touch_motion(stream, slot, x, y))
    }

    async fn notify_touch_up(&self, session_handle: ObjectPath<'_>, options: Options, slot: u32) -> fdo::Result<()> {
        notify!(self, session_handle, options, "NotifyTouchUp", (slot), m => m.touch_up(slot))
    }

    /// Solo las sesiones reenviadas a GNOME soportan libei; las de Mutter usan los Notify*.
    async fn connect_to_eis(&self, session_handle: ObjectPath<'_>, app_id: &str, options: Options) -> fdo::Result<zbus::zvariant::OwnedFd> {
        let st = &self.state;
        match st.get(session_handle.as_str()).await {
            Some(entry) if matches!(entry.kind, Kind::Forward) => {
                let reply = st
                    .conn
                    .call_method(Some(forward::GNOME_DEST), PORTAL_PATH, Some(REMOTEDESKTOP_IFACE), "ConnectToEIS", &(session_handle, app_id, options))
                    .await
                    .map_err(|e| fdo::Error::Failed(format!("backend de GNOME: {e}")))?;
                reply.body().deserialize::<zbus::zvariant::OwnedFd>().map_err(|e| fdo::Error::Failed(e.to_string()))
            }
            _ => Err(fdo::Error::NotSupported("ConnectToEIS no esta disponible en sesiones de rdbridge".into())),
        }
    }
}
