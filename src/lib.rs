//! TURRON — Transparent User Remote Rendering & Output Negotiator.
//!
//! Backend de xdg-desktop-portal (ScreenCast + RemoteDesktop) para GNOME. El consentimiento
//! es una **politica explicita del dueño** (allowlist en `~/.config/turron/policy.toml`,
//! administrada con `turronctl`): quien no esta en la lista pasa por el dialogo normal de GNOME.

pub mod caller;
pub mod control;
pub mod forward;
pub mod mutter;
pub mod policy;
pub mod remotedesktop;
pub mod screencast;
pub mod session;
pub mod state;

/// Nombre de bus del backend (el mismo que declara `turron.portal`).
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.turron";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const CONTROL_PATH: &str = "/org/turron/Control";
pub const CONTROL_IFACE: &str = "org.turron.Control1";
