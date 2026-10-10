//! org.freedesktop.impl.portal.ScreenCast

use crate::forward::{self, PortalResult, PORTAL_PATH, SCREENCAST_IFACE};
use crate::mutter::mutter_cursor_mode;
use crate::session::{create_if_allowed, opt_u32, register_forward, start_results, Created};
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
        let caller = match create_if_allowed(st, handle.as_str(), sh).await? {
            Created::Approved => return Ok((RESPONSE_OK, HashMap::new())),
            Created::Forward(c) => c,
        };
        let r = forward::call_result(&st.conn, PORTAL_PATH, SCREENCAST_IFACE, "CreateSession", &(handle, session_handle.clone(), app_id, options)).await?;
        if r.0 == RESPONSE_OK {
            register_forward(st, sh, caller).await?;
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
                match m.lock().await.record_monitor(mutter_cursor_mode(cursor), entry.monitor.as_deref().unwrap_or("")).await {
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
