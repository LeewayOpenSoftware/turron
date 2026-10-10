//! rdbridge: backend de xdg-desktop-portal (ScreenCast + RemoteDesktop) para GNOME.
//!
//! Cuando quien pide la captura es RustDesk, la sesion se crea sin dialogo con la API privada
//! de Mutter (la misma que usa gnome-remote-desktop). Para cualquier otro programa las
//! llamadas se reenvian tal cual al backend de GNOME (`xdg-desktop-portal-gnome`).

mod caller;
mod forward;
mod mutter;
mod remotedesktop;
mod screencast;
mod session;
mod state;

use anyhow::{Context, Result};
use state::State;
use std::sync::Arc;
use zbus::connection::Builder;

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.rdbridge";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("RDBRIDGE_LOG").unwrap_or_else(|_| "rdbridge=info".into()))
        .with_writer(std::io::stderr)
        .init();

    let conn = Builder::session()
        .context("no hay bus de sesion (DBUS_SESSION_BUS_ADDRESS)")?
        .build()
        .await
        .context("no se pudo conectar al bus de sesion")?;
    let state = Arc::new(State::new(conn.clone()));

    let server = conn.object_server();
    server.at(OBJECT_PATH, screencast::ScreenCast::new(state.clone())).await?;
    server.at(OBJECT_PATH, remotedesktop::RemoteDesktop::new(state.clone())).await?;
    conn.request_name(BUS_NAME).await.with_context(|| format!("no se pudo registrar {BUS_NAME} (¿ya hay otra instancia?)"))?;
    tracing::info!("rdbridge listo como {BUS_NAME}; permitidos: {:?}", caller::allowed_prefixes());

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("rdbridge terminando");
    state.shutdown().await;
    Ok(())
}
