//! org.freedesktop.impl.portal.ScreenCast

use crate::caller;
use crate::forward::{self, PortalResult, PORTAL_PATH, SCREENCAST_IFACE};
use crate::mutter::{mutter_cursor_mode, MutterSession};
use crate::session::{export_session, opt_u32, start_results};
use crate::state::{Kind, State};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::{fdo, interface, zvariant::{ObjectPath, OwnedValue}};

pub struct ScreenCast {
    state: Arc<State>,
}

impl ScreenCast {
    pub fn new(state: Arc<State>) -> Self {
        ScreenCast { state }
    }
}

type Options = HashMap<String, OwnedValue>;

const RESPONSE_OK: u32 = 0;
const RESPONSE_OTHER: u32 = 2;

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    #[zbus(property)]
    async fn available_source_types(&self) -> u32 {
        3
    }

    #[zbus(property)]
    async fn available_cursor_modes(&self) -> u32 {
        7
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        5
    }

    async fn create_session(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, options: Options) -> fdo::Result<PortalResult> {
        let (st, sh) = (&self.state, session_handle.as_str());
        if caller::is_trusted(&st.conn, handle.as_str()).await {
            match MutterSession::create(&st.conn).await {
                Ok(m) => {
                    st.insert(sh, Kind::Mutter(tokio::sync::Mutex::new(m))).await;
                    export_session(st, sh).await?;
                    return Ok((RESPONSE_OK, HashMap::new()));
                }
                Err(e) => {
                    // Sin API de Mutter se cae al backend de GNOME (con su dialogo) en vez de fallar.
                    tracing::error!("no se pudo crear la sesion de Mutter ({e:#}); se reenvia a GNOME");
                }
            }
        }
        let r = forward::call_result(&st.conn, PORTAL_PATH, SCREENCAST_IFACE, "CreateSession", &(handle, session_handle.clone(), app_id, options)).await?;
        if r.0 == RESPONSE_OK {
            st.insert(sh, Kind::Forward).await;
            export_session(st, sh).await?;
        }
        Ok(r)
    }

    async fn select_sources(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, options: Options) -> fdo::Result<PortalResult> {
        let st = &self.state;
        let Some(entry) = st.get(session_handle.as_str()).await else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        match &entry.kind {
            Kind::Forward => forward::call_result(&st.conn, PORTAL_PATH, SCREENCAST_IFACE, "SelectSources", &(handle, session_handle, app_id, options)).await,
            Kind::Mutter(m) => {
                let cursor = opt_u32(&options, "cursor_mode");
                *entry.cursor_mode.lock().await = cursor;
                if let Some(p) = opt_u32(&options, "persist_mode") {
                    *entry.persist_mode.lock().await = p;
                }
                // Se pide el stream ya: la senal de PipeWire se escucha antes del Start.
                match m.lock().await.record_monitor(mutter_cursor_mode(cursor)).await {
                    Ok(()) => Ok((RESPONSE_OK, HashMap::new())),
                    Err(e) => {
                        tracing::error!("SelectSources: {e:#}");
                        Ok((RESPONSE_OTHER, HashMap::new()))
                    }
                }
            }
        }
    }

    async fn start(&self, handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: &str, parent_window: &str, options: Options) -> fdo::Result<PortalResult> {
        let st = &self.state;
        let Some(entry) = st.get(session_handle.as_str()).await else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        match &entry.kind {
            Kind::Forward => forward::call_result(&st.conn, PORTAL_PATH, SCREENCAST_IFACE, "Start", &(handle, session_handle, app_id, parent_window, options)).await,
            Kind::Mutter(m) => {
                let mut m = m.lock().await;
                match m.start().await {
                    Ok(streams) => {
                        let persist = *entry.persist_mode.lock().await;
                        tracing::info!("ScreenCast.Start sin dialogo: {} stream(s) {:?}", streams.len(), streams.iter().map(|s| s.node_id).collect::<Vec<_>>());
                        Ok((RESPONSE_OK, start_results(&streams, persist)))
                    }
                    Err(e) => {
                        tracing::error!("ScreenCast.Start: {e:#}");
                        Ok((RESPONSE_OTHER, HashMap::new()))
                    }
                }
            }
        }
    }
}
