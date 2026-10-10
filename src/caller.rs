//! Identificacion de quien pidio la captura.
//!
//! El portal le pasa al backend la ruta del Request/Session, que incluye el nombre unico del
//! llamador original (`/org/freedesktop/portal/desktop/request/1_234/token` <-> `:1.234`).
//! Con ese nombre se le pide al bus el PID y se mira `/proc/<pid>/exe`.

use zbus::{fdo::DBusProxy, names::BusName, Connection};

/// Prefijos de ejecutable que se aprueban sin dialogo. `RDBRIDGE_ALLOW` agrega mas
/// (separados por `:`), solo para pruebas.
pub fn allowed_prefixes() -> Vec<String> {
    let mut v: Vec<String> = vec!["/usr/share/rustdesk/".into(), "/usr/lib/rustdesk/".into(), "/usr/bin/rustdesk".into()];
    if let Ok(extra) = std::env::var("RDBRIDGE_ALLOW") {
        v.extend(extra.split(':').filter(|s| !s.is_empty()).map(str::to_owned));
    }
    v
}

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

fn exe_is_allowed(exe: &str, allowed: &[String]) -> bool {
    // Tras una actualizacion el kernel agrega " (deleted)" al enlace.
    let exe = exe.trim_end_matches(" (deleted)");
    allowed.iter().any(|p| if p.ends_with('/') { exe.starts_with(p.as_str()) } else { exe == p })
}

/// `true` si el proceso que origino la llamada es una de las aplicaciones permitidas.
pub async fn is_trusted(conn: &Connection, handle: &str) -> bool {
    let Some(name) = unique_name_from_handle(handle) else {
        tracing::debug!("handle sin remitente reconocible: {handle}");
        return false;
    };
    let result: zbus::Result<u32> = async {
        let bus = DBusProxy::new(conn).await?;
        let bus_name = BusName::try_from(name.as_str())?;
        Ok(bus.get_connection_unix_process_id(bus_name).await?)
    }
    .await;
    let pid = match result {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("no se pudo obtener el PID de {name}: {e}");
            return false;
        }
    };
    let exe = match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            tracing::warn!("no se pudo leer /proc/{pid}/exe: {e}");
            return false;
        }
    };
    let trusted = exe_is_allowed(&exe, &allowed_prefixes());
    tracing::info!("llamador {name} pid {pid} exe {exe}: {}", if trusted { "aprobado sin dialogo" } else { "se reenvia a GNOME" });
    trusted
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

    #[test]
    fn exe_matching() {
        let a = vec!["/usr/share/rustdesk/".to_string(), "/usr/bin/rustdesk".to_string()];
        assert!(exe_is_allowed("/usr/share/rustdesk/rustdesk", &a));
        assert!(exe_is_allowed("/usr/bin/rustdesk (deleted)", &a));
        assert!(!exe_is_allowed("/usr/bin/rustdesk-evil", &a));
        assert!(!exe_is_allowed("/home/x/usr/share/rustdesk/rustdesk", &a));
    }
}
