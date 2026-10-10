//! Reenvio de llamadas al backend de GNOME para los llamadores que no son RustDesk.

use std::collections::HashMap;
use zbus::{fdo, zvariant::{DynamicType, OwnedValue}, Connection};

pub const GNOME_DEST: &str = "org.freedesktop.impl.portal.desktop.gnome";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCREENCAST_IFACE: &str = "org.freedesktop.impl.portal.ScreenCast";
pub const REMOTEDESKTOP_IFACE: &str = "org.freedesktop.impl.portal.RemoteDesktop";
pub const SESSION_IFACE: &str = "org.freedesktop.impl.portal.Session";

pub type PortalResult = (u32, HashMap<String, OwnedValue>);

fn failed(e: zbus::Error) -> fdo::Error {
    fdo::Error::Failed(format!("backend de GNOME: {e}"))
}

/// Llama a un metodo del backend de GNOME que devuelve `(u response, a{sv} results)`.
pub async fn call_result<B>(conn: &Connection, path: &str, iface: &str, method: &str, body: &B) -> fdo::Result<PortalResult>
where
    B: serde::Serialize + DynamicType,
{
    let reply = conn.call_method(Some(GNOME_DEST), path, Some(iface), method, body).await.map_err(failed)?;
    reply.body().deserialize::<PortalResult>().map_err(|e| fdo::Error::Failed(format!("respuesta ilegible de GNOME: {e}")))
}

/// Llama a un metodo del backend de GNOME sin valor de retorno.
pub async fn call_unit<B>(conn: &Connection, path: &str, iface: &str, method: &str, body: &B) -> fdo::Result<()>
where
    B: serde::Serialize + DynamicType,
{
    conn.call_method(Some(GNOME_DEST), path, Some(iface), method, body).await.map(|_| ()).map_err(failed)
}
