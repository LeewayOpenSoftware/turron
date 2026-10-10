//! turrond: demonio de TURRON (backend de xdg-desktop-portal para GNOME).
//!
//! Aprueba sin dialogo SOLO a los ejecutables que el dueño autorizo con `turronctl allow`;
//! todo lo demas se reenvia al backend de GNOME (dialogo normal).

use anyhow::{Context, Result};
use std::sync::Arc;
use turron::state::State;
use turron::{control, policy, remotedesktop, screencast, BUS_NAME, CONTROL_PATH, OBJECT_PATH};
use zbus::connection::Builder;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("TURRON_LOG").unwrap_or_else(|_| "turron=info".into()))
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
    server.at(CONTROL_PATH, control::Control { state: state.clone() }).await?;
    conn.request_name(BUS_NAME).await.with_context(|| format!("no se pudo registrar {BUS_NAME} (¿ya hay otra instancia?)"))?;

    let path = policy::policy_path();
    match policy::Policy::load(&path) {
        Ok(p) => tracing::info!("turrond listo como {BUS_NAME}; politica {} con {} entrada(s) permitida(s)", path.display(), p.allow.len()),
        Err(e) => tracing::warn!("turrond listo, pero la politica no es utilizable (todo se reenvia a GNOME): {e:#}"),
    }

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("turrond terminando");
    state.shutdown().await;
    Ok(())
}
