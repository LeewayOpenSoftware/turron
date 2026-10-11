//! Objeto `org.freedesktop.impl.portal.Session` que el portal usa para cerrar una sesion,
//! y utilidades para armar los resultados que el portal espera.

use crate::caller;
use crate::forward;
use crate::mutter::{MutterSession, StreamInfo};
use crate::policy::{Caller, Decision};
use crate::state::{Kind, SessionInfo, State};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::{
    fdo, interface,
    zvariant::{Array, ObjectPath, OwnedValue, Signature, StructureBuilder, Value},
};

pub struct Session {
    pub state: Arc<State>,
    pub handle: String,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(&self) -> fdo::Result<()> {
        tracing::info!("Session.Close {}", self.handle);
        close_session(&self.state, &self.handle).await;
        Ok(())
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        1
    }
}

/// Cierra la sesion (Mutter o GNOME), la olvida y quita el objeto exportado.
pub async fn close_session(state: &Arc<State>, handle: &str) {
    if let Some(entry) = state.remove(handle).await {
        match &entry.kind {
            Kind::Mutter(m) => m.lock().await.stop().await,
            Kind::Forward => {
                let _ = forward::call_unit(&state.conn, handle, forward::SESSION_IFACE, "Close", &()).await;
            }
        }
    }
    if let Ok(path) = ObjectPath::try_from(handle) {
        let _ = state.conn.object_server().remove::<Session, _>(path).await;
    }
}

/// Exporta el objeto de sesion en la ruta que dio el portal.
pub async fn export_session(state: &Arc<State>, handle: &str) -> fdo::Result<()> {
    let path = ObjectPath::try_from(handle).map_err(|e| fdo::Error::InvalidArgs(e.to_string()))?;
    state
        .conn
        .object_server()
        .at(path, Session { state: state.clone(), handle: handle.to_owned() })
        .await
        .map_err(|e| fdo::Error::Failed(e.to_string()))?;
    Ok(())
}

pub fn opt_u32(options: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    options.get(key)?.downcast_ref::<u32>().ok()
}

fn pair(a: i32, b: i32) -> Value<'static> {
    Value::from(StructureBuilder::new().add_field(a).add_field(b).build().expect("estructura (ii)"))
}

fn stream_value(s: &StreamInfo) -> Value<'static> {
    let mut props: HashMap<String, Value<'static>> = HashMap::new();
    props.insert("position".into(), pair(s.position.0, s.position.1));
    props.insert("size".into(), pair(s.size.0, s.size.1));
    // 1 = monitor (el portal define 1 monitor, 2 ventana, 4 virtual).
    props.insert("source_type".into(), Value::from(1u32));
    Value::from(StructureBuilder::new().add_field(s.node_id).add_field(props).build().expect("estructura (ua{sv})"))
}

/// Resultados de Start: `streams` (a(ua{sv})) y, si se pidio, los datos de persistencia.
/// `restore_data` no guarda nada util (turron nunca pregunta) pero hace que el portal
/// entregue un `restore_token` y la aplicacion no vuelva a pedir permiso.
pub fn start_results(streams: &[StreamInfo], persist_mode: u32) -> HashMap<String, OwnedValue> {
    let mut out: HashMap<String, OwnedValue> = HashMap::new();
    if !streams.is_empty() {
        let sig = Signature::try_from("(ua{sv})").expect("firma de stream");
        let mut arr = Array::new(&sig);
        for s in streams {
            if arr.append(stream_value(s)).is_err() {
                tracing::error!("no se pudo armar el stream");
            }
        }
        if let Ok(v) = OwnedValue::try_from(Value::from(arr)) {
            out.insert("streams".into(), v);
        }
    }
    if persist_mode > 0 {
        if let Ok(v) = OwnedValue::try_from(Value::from(persist_mode)) {
            out.insert("persist_mode".into(), v);
        }
        let data = StructureBuilder::new().add_field("turron").add_field(1u32).add_field(Value::from("primary")).build();
        if let Ok(data) = data {
            if let Ok(v) = OwnedValue::try_from(Value::from(data)) {
                out.insert("restore_data".into(), v);
            }
        }
    }
    out
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Resultado de decidir un CreateSession: o se creo la sesion de Mutter, o hay que reenviar.
pub enum Created {
    Approved,
    Forward(Option<Caller>),
}

/// Consulta la politica y, si el llamador esta autorizado, crea la sesion de Mutter sin dialogo.
/// En cualquier otro caso (o si Mutter falla) devuelve `Forward` para que se use el backend de GNOME.
pub async fn create_if_allowed(state: &Arc<State>, request_handle: &str, session_handle: &str) -> fdo::Result<Created> {
    let (caller_info, decision) = caller::authorize(&state.conn, request_handle).await;
    if let (Some(c), Decision::Approve { name, monitor, .. }) = (&caller_info, &decision) {
        match MutterSession::create(&state.conn).await {
            Ok(m) => {
                let info = SessionInfo { name: name.clone(), pid: c.pid, exe: c.exe.clone(), since: now() };
                state.insert(session_handle, Kind::Mutter(tokio::sync::Mutex::new(m)), info, monitor.clone()).await;
                export_session(state, session_handle).await?;
                return Ok(Created::Approved);
            }
            Err(e) => {
                let text = format!("{e:#}");
                if text.contains("inhibited") {
                    tracing::warn!("Mutter no permite crear sesiones remotas ahora (pantalla bloqueada o acceso remoto inhibido): {text}; se reenvia a GNOME");
                } else {
                    tracing::error!("no se pudo crear la sesion de Mutter ({text}); se reenvia a GNOME");
                }
            }
        }
    }
    Ok(Created::Forward(caller_info))
}

/// Registra una sesion cuyo CreateSession ya fue aceptado por el backend de GNOME.
pub async fn register_forward(state: &Arc<State>, session_handle: &str, caller: Option<Caller>) -> fdo::Result<()> {
    let info = SessionInfo {
        name: "gnome".into(),
        pid: caller.as_ref().map_or(0, |c| c.pid),
        exe: caller.map(|c| c.exe).unwrap_or_default(),
        since: now(),
    };
    state.insert(session_handle, Kind::Forward, info, None).await;
    export_session(state, session_handle).await
}
