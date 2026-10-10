//! Estado compartido: sesiones activas (propias de Mutter o reenviadas a GNOME).

use crate::mutter::MutterSession;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::Connection;

pub enum Kind {
    /// Sesion creada por rdbridge con la API de Mutter (llamador aprobado).
    Mutter(Mutex<MutterSession>),
    /// Sesion que vive en el backend de GNOME; rdbridge solo reenvia.
    Forward,
}

pub struct Entry {
    pub kind: Kind,
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

    pub async fn insert(&self, handle: &str, kind: Kind) -> Arc<Entry> {
        let entry = Arc::new(Entry { kind, persist_mode: Mutex::new(0), device_types: Mutex::new(7), cursor_mode: Mutex::new(None) });
        self.sessions.lock().await.insert(handle.to_owned(), entry.clone());
        entry
    }

    pub async fn get(&self, handle: &str) -> Option<Arc<Entry>> {
        self.sessions.lock().await.get(handle).cloned()
    }

    pub async fn remove(&self, handle: &str) -> Option<Arc<Entry>> {
        self.sessions.lock().await.remove(handle)
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
