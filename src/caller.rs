//! Identificacion de quien pidio la captura.
//!
//! El portal le pasa al backend la ruta del Request/Session, que incluye el nombre unico del
//! llamador original (`/org/freedesktop/portal/desktop/request/1_234/token` <-> `:1.234`).
//! Con ese nombre se le pide al bus el PID y se mira `/proc/<pid>/exe`. La decision de
//! aprobar o no la toma `policy::decide` (allowlist del dueño), no este modulo.

use crate::policy::{decide, Caller, Decision, Dpkg, Policy};
use std::path::PathBuf;
use zbus::{fdo::DBusProxy, names::BusName, Connection};

/// Nombre unico del llamador a partir de la ruta de Request/Session del portal.
pub fn unique_name_from_handle(handle: &str) -> Option<String> {
    let mut parts = handle.rsplit('/');
    let _token = parts.next()?;
    let sender = parts.next()?;
    if sender.is_empty() || !sender.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    Some(format!(":{}", sender.replace('_', ".")))
}

/// Proceso que origino la llamada, si se puede determinar.
pub async fn identify(conn: &Connection, handle: &str) -> Option<Caller> {
    let name = unique_name_from_handle(handle)?;
    let pid: zbus::Result<u32> = async {
        let bus = DBusProxy::new(conn).await?;
        Ok(bus.get_connection_unix_process_id(BusName::try_from(name.as_str())?).await?)
    }
    .await;
    let pid = match pid {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("no se pudo obtener el PID de {name}: {e}");
            return None;
        }
    };
    let content_path = PathBuf::from(format!("/proc/{pid}/exe"));
    let exe = match std::fs::read_link(&content_path) {
        Ok(p) => p.to_string_lossy().trim_end_matches(" (deleted)").to_owned(),
        Err(e) => {
            tracing::warn!("no se pudo leer /proc/{pid}/exe: {e}");
            return None;
        }
    };
    Some(Caller { pid, exe, content_path })
}

/// Identifica al llamador y consulta la politica. Todo lo que no sea una aprobacion
/// explicita termina en `Forward` (dialogo normal de GNOME).
pub async fn authorize(conn: &Connection, handle: &str) -> (Option<Caller>, Decision) {
    let Some(caller) = identify(conn, handle).await else {
        return (None, Decision::Forward { reason: "llamador no identificable".into() });
    };
    let policy = match Policy::load(&crate::policy::policy_path()) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("politica no utilizable, se reenvia a GNOME: {e:#}");
            Policy::default()
        }
    };
    let decision = decide(&policy, &caller, &Dpkg::default());
    match &decision {
        Decision::Approve { name, reason, .. } => tracing::info!("APROBADO sin dialogo: {name} (pid {}, {}): {reason}", caller.pid, caller.exe),
        Decision::Forward { reason } => tracing::info!("REENVIADO a GNOME: pid {} {}: {reason}", caller.pid, caller.exe),
    }
    (Some(caller), decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_to_unique_name() {
        assert_eq!(unique_name_from_handle("/org/freedesktop/portal/desktop/request/1_234/rd1").as_deref(), Some(":1.234"));
        assert_eq!(unique_name_from_handle("/org/freedesktop/portal/desktop/session/2_7/s").as_deref(), Some(":2.7"));
        assert_eq!(unique_name_from_handle("/x/y"), None);
        assert_eq!(unique_name_from_handle("/org/freedesktop/portal/desktop/request/abc/t"), None);
    }
}
