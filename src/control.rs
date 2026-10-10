//! Interfaz de administracion `org.turron.Control1` (la usa `turronctl status/cut`).
//! La politica NO se modifica por D-Bus: solo el archivo del dueño (permisos 600) la define.

use crate::session::close_session;
use crate::state::State;
use std::sync::Arc;
use zbus::interface;

pub struct Control {
    pub state: Arc<State>,
}

#[interface(name = "org.turron.Control1")]
impl Control {
    /// Sesiones activas: (handle, nombre en la politica, pid, ejecutable, desde (Unix), tipo).
    async fn list_sessions(&self) -> Vec<(String, String, u32, String, u64, String)> {
        let mut v: Vec<_> = self
            .state
            .list()
            .await
            .into_iter()
            .map(|(h, i, mutter)| (h, i.name, i.pid, i.exe, i.since, if mutter { "turron".to_owned() } else { "gnome".to_owned() }))
            .collect();
        v.sort_by_key(|s| s.4);
        v
    }

    /// Corta una sesion. Devuelve `false` si no existe.
    async fn cut(&self, handle: &str) -> bool {
        if self.state.get(handle).await.is_none() {
            return false;
        }
        tracing::warn!("CORTE manual de la sesion {handle} (turronctl)");
        close_session(&self.state, handle).await;
        true
    }
}
