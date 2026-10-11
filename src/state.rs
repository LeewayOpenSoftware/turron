//! Estado compartido: sesiones activas (propias de Mutter o reenviadas a GNOME).

use crate::mutter::MutterSession;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::Connection;

pub enum Kind {
    /// Sesion creada por turron con la API de Mutter (llamador aprobado).
    Mutter(Mutex<MutterSession>),
    /// Sesion que vive en el backend de GNOME; turron solo reenvia.
    Forward,
}

/// Datos de la sesion para `turronctl status`.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Entrada de la politica que la aprobo (o "gnome" si la maneja el backend de GNOME).
    pub name: String,
    pub pid: u32,
    pub exe: String,
    pub since: u64,
}

pub struct Entry {
    pub kind: Kind,
    pub info: SessionInfo,
    /// Conector de monitor fijado por la politica (None = primario).
    pub monitor: Option<String>,
    /// `persist_mode` pedido en SelectSources/SelectDevices (0 = no persistir).
    pub persist_mode: Mutex<u32>,
    /// Tipos de dispositivo pedidos en SelectDevices (para informarlos en Start).
    pub device_types: Mutex<u32>,
    /// Tipo de cursor pedido (modo del portal) para RecordMonitor.
    pub cursor_mode: Mutex<Option<u32>>,
}

pub struct State {
    pub conn: Connection,
    sessions: Mutex<HashMap<String, Arc<Entry>>>,
}

impl State {
    pub fn new(conn: Connection) -> Self {
        State { conn, sessions: Mutex::new(HashMap::new()) }
    }

    pub async fn insert(&self, handle: &str, kind: Kind, info: SessionInfo, monitor: Option<String>) -> Arc<Entry> {
        let entry = Arc::new(Entry { kind, info, monitor, persist_mode: Mutex::new(0), device_types: Mutex::new(7), cursor_mode: Mutex::new(None) });
        self.sessions.lock().await.insert(handle.to_owned(), entry.clone());
        entry
    }

    pub async fn get(&self, handle: &str) -> Option<Arc<Entry>> {
        self.sessions.lock().await.get(handle).cloned()
    }

    pub async fn remove(&self, handle: &str) -> Option<Arc<Entry>> {
        self.sessions.lock().await.remove(handle)
    }

    pub async fn list(&self) -> Vec<(String, SessionInfo, bool)> {
        self.sessions.lock().await.iter().map(|(h, e)| (h.clone(), e.info.clone(), matches!(e.kind, Kind::Mutter(_)))).collect()
    }

    /// Cierra las sesiones de Mutter abiertas (al terminar el servicio).
    pub async fn shutdown(&self) {
        let all: Vec<_> = self.sessions.lock().await.drain().map(|(_, e)| e).collect();
        for e in all {
            if let Kind::Mutter(m) = &e.kind {
                m.lock().await.stop().await;
            }
        }
    }
}
