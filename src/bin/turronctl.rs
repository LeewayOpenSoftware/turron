//! turronctl: administra la politica de consentimiento de TURRON y muestra/corta sesiones.
//!
//! La politica es un archivo del dueño (`~/.config/turron/policy.toml`, permisos 600).
//! Nada se autoriza solo: cada entrada la crea el dueño con `turronctl allow`.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use turron::policy::{self, Dpkg, Policy};
use turron::{BUS_NAME, CONTROL_IFACE, CONTROL_PATH};

const USAGE: &str = "turronctl - consentimiento de TURRON

  turronctl allow <paquete|ruta> [--monitor CONECTOR] [--name NOMBRE]
        autoriza a un programa a capturar/controlar la pantalla SIN dialogo
        (paquete dpkg, p. ej. rustdesk; o ruta a un ejecutable)
  turronctl deny <ruta|nombre>     bloquea (siempre dialogo de GNOME)
  turronctl revoke <nombre|ruta>   quita la autorizacion o el bloqueo
  turronctl list                   muestra la politica
  turronctl status                 muestra el servicio y las sesiones activas
  turronctl cut <n|handle|all>     corta una sesion activa (n = numero de `status`)
  turronctl path                   muestra el archivo de politica";

fn main() {
    if let Err(e) = run() {
        eprintln!("turronctl: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let file = policy::policy_path();
    let dpkg = Dpkg::default();
    match cmd {
        "allow" => {
            let target = args.get(1).context("falta el paquete o la ruta (turronctl allow rustdesk)")?;
            let monitor = flag(&args, "--monitor");
            let name = flag(&args, "--name");
            let exes: Vec<PathBuf> = if target.contains('/') {
                vec![PathBuf::from(target)]
            } else {
                let found = policy::package_executables(&dpkg, target);
                if found.is_empty() {
                    bail!("el paquete dpkg '{target}' no existe o no tiene un ejecutable llamado '{target}'; pasa la ruta del ejecutable");
                }
                found
            };
            let mut p = Policy::load(&file).unwrap_or_default();
            for exe in exes {
                let entry = policy::entry_for(&exe, &dpkg, monitor.clone(), name.clone().or_else(|| if target.contains('/') { None } else { Some(target.clone()) }))?;
                println!("AUTORIZADO: {} -> {}{}", entry.name, entry.exe, entry.package.as_ref().map(|p| format!(" (paquete {p}: se aceptan sus actualizaciones verificadas por dpkg)")).unwrap_or_default());
                println!("  sha256 {}", entry.sha256);
                p.add(entry);
            }
            p.save(&file)?;
            println!("Politica guardada en {} (permisos 600). Se aplica a la proxima sesion.", file.display());
        }
        "deny" => {
            let target = args.get(1).context("falta la ruta o el nombre")?;
            let mut p = Policy::load(&file).unwrap_or_default();
            let exe = p.allow.iter().find(|e| &e.name == target || &e.exe == target).map(|e| e.exe.clone()).unwrap_or_else(|| target.clone());
            p.deny_exe(&exe);
            p.save(&file)?;
            println!("BLOQUEADO: {exe}");
        }
        "revoke" => {
            let target = args.get(1).context("falta el nombre o la ruta")?;
            let mut p = Policy::load(&file).unwrap_or_default();
            let n = p.revoke(target);
            p.save(&file)?;
            println!("{n} entrada(s) quitada(s)");
        }
        "list" => {
            let p = Policy::load(&file)?;
            if p.allow.is_empty() && p.deny.is_empty() {
                println!("Politica vacia: TURRON no aprueba nada, todo pasa por el dialogo de GNOME.");
            }
            for e in &p.allow {
                println!("PERMITIDO {:<12} {}  sha256 {}…{}{}{}", e.name, e.exe, &e.sha256[..12.min(e.sha256.len())], "", e.package.as_ref().map(|p| format!("  paquete {p}")).unwrap_or_default(), e.monitor.as_ref().map(|m| format!("  monitor {m}")).unwrap_or_default());
            }
            for d in &p.deny {
                println!("BLOQUEADO {d}");
            }
        }
        "path" => println!("{}", file.display()),
        "status" => status(&file, false, None)?,
        "cut" => status(&file, true, args.get(1).map(String::as_str))?,
        "" | "help" | "-h" | "--help" => println!("{USAGE}"),
        other => bail!("comando desconocido: {other}\n\n{USAGE}"),
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

type SessionRow = (String, String, u32, String, u64, String);

fn status(file: &std::path::Path, cut: bool, which: Option<&str>) -> Result<()> {
    let policy_line = match Policy::load(file) {
        Ok(p) => format!("{} permitido(s), {} bloqueado(s)", p.allow.len(), p.deny.len()),
        Err(e) => format!("NO UTILIZABLE ({e:#})"),
    };
    let conn = zbus::blocking::Connection::session().context("sin bus de sesion")?;
    let proxy = zbus::blocking::Proxy::new(&conn, BUS_NAME, CONTROL_PATH, CONTROL_IFACE)?;
    let running = zbus::blocking::fdo::DBusProxy::new(&conn)?
        .name_has_owner(BUS_NAME.try_into()?)
        .unwrap_or(false);
    if !cut {
        println!("Servicio turrond: {}", if running { "en marcha" } else { "detenido (se inicia solo cuando el portal lo necesita)" });
        println!("Politica: {} -> {policy_line}", file.display());
    }
    if !running {
        if cut {
            bail!("turrond no esta en marcha: no hay sesiones que cortar");
        }
        return Ok(());
    }
    let rows: Vec<SessionRow> = proxy.call("ListSessions", &())?;
    if !cut {
        if rows.is_empty() {
            println!("Sesiones activas: ninguna");
        }
        for (i, (handle, name, pid, exe, since, kind)) in rows.iter().enumerate() {
            let age = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).saturating_sub(*since);
            println!("  [{}] {} {} (pid {pid}, {exe}) hace {age}s  {handle}", i + 1, if kind == "turron" { "SIN DIALOGO" } else { "via GNOME  " }, name);
        }
        return Ok(());
    }
    let which = which.context("turronctl cut <n|handle|all>")?;
    let targets: Vec<&SessionRow> = if which == "all" {
        rows.iter().collect()
    } else if let Ok(n) = which.parse::<usize>() {
        rows.get(n.wrapping_sub(1)).into_iter().collect()
    } else {
        rows.iter().filter(|r| r.0 == which).collect()
    };
    if targets.is_empty() {
        bail!("no hay una sesion que coincida con '{which}'");
    }
    for r in targets {
        let ok: bool = proxy.call("Cut", &(r.0.as_str(),))?;
        println!("{} sesion {} ({})", if ok { "cortada" } else { "no se pudo cortar" }, r.0, r.1);
    }
    Ok(())
}
