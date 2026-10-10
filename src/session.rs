//! Objeto `org.freedesktop.impl.portal.Session` que el portal usa para cerrar una sesion,
//! y utilidades para armar los resultados que el portal espera.

use crate::forward;
use crate::mutter::StreamInfo;
use crate::state::{Kind, State};
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
/// `restore_data` no guarda nada util (rdbridge nunca pregunta) pero hace que el portal
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
        let data = StructureBuilder::new().add_field("rdbridge").add_field(1u32).add_field(Value::from("primary")).build();
        if let Ok(data) = data {
            if let Ok(v) = OwnedValue::try_from(Value::from(data)) {
                out.insert("restore_data".into(), v);
            }
        }
    }
    out
}
