//! Politica de consentimiento: allowlist persistente del dueño.
//!
//! Regla de oro: **sin entrada en la allowlist no se aprueba nada** (la llamada se reenvia al
//! backend de GNOME, que muestra su dialogo). Nada se agrega solo: las entradas las crea el
//! dueño con `turronctl allow`.
//!
//! Una entrada identifica un ejecutable por su ruta real y el SHA-256 de su contenido.
//! Si el binario cambio de contenido, solo se sigue aprobando cuando la entrada tiene un
//! paquete dpkg y el binario coincide con el `md5sums` que dpkg instalo para ese paquete
//! (actualizacion legitima); cualquier otro cambio vuelve a pedir el dialogo.

use anyhow::{bail, Context, Result};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Nombre corto para `turronctl list/revoke` (por defecto el nombre del ejecutable).
    pub name: String,
    /// Ruta real del ejecutable (sin symlinks).
    pub exe: String,
    /// SHA-256 del binario cuando se autorizo.
    pub sha256: String,
    /// Paquete dpkg dueño del binario (permite aceptar sus actualizaciones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Conector del monitor a compartir (por defecto el primario).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    /// Segundos Unix en que se agrego.
    #[serde(default)]
    pub added: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    #[serde(default)]
    pub allow: Vec<Entry>,
    /// Ejecutables bloqueados explicitamente (siempre pasan por el dialogo de GNOME).
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Quien hace la llamada (ya identificado por el demonio).
#[derive(Debug, Clone)]
pub struct Caller {
    pub pid: u32,
    /// Ruta real del ejecutable (`/proc/<pid>/exe` sin " (deleted)").
    pub exe: String,
    /// Donde leer el contenido: `/proc/<pid>/exe` sirve aunque el binario ya se haya reemplazado.
    pub content_path: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Se aprueba sin dialogo, por la entrada `name`; `monitor` = conector pedido en la politica.
    Approve { name: String, monitor: Option<String>, reason: String },
    /// Se reenvia al backend de GNOME (dialogo normal).
    Forward { reason: String },
}

pub fn policy_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TURRON_POLICY") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("turron").join("policy.toml")
}

impl Policy {
    /// Lee la politica. Si no existe es una politica vacia (todo se reenvia). Si el archivo
    /// puede ser modificado por otros usuarios (permisos distintos de 600) NO se confia en el.
    pub fn load(path: &Path) -> Result<Policy> {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Policy::default()),
            Err(e) => return Err(e).with_context(|| format!("no se pudo leer {}", path.display())),
        };
        if meta.permissions().mode() & 0o077 != 0 {
            bail!("{} tiene permisos {:o}: debe ser 600; la politica se ignora (todo se reenvia a GNOME)", path.display(), meta.permissions().mode() & 0o777);
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).with_context(|| format!("politica ilegible en {}", path.display()))
    }

    /// Guarda con permisos 600 (de forma atomica).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let text = format!("# TURRON: politica de consentimiento del dueño. Se edita con `turronctl`.\n{}", toml::to_string_pretty(self)?);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn add(&mut self, entry: Entry) {
        self.deny.retain(|d| d != &entry.exe);
        self.allow.retain(|e| e.exe != entry.exe);
        self.allow.push(entry);
    }

    /// Quita las entradas cuyo nombre o ruta coincidan (y los bloqueos de esa ruta).
    pub fn revoke(&mut self, name_or_exe: &str) -> usize {
        let before = self.allow.len() + self.deny.len();
        let exes: Vec<String> = self.allow.iter().filter(|e| e.name == name_or_exe || e.exe == name_or_exe).map(|e| e.exe.clone()).collect();
        self.allow.retain(|e| e.name != name_or_exe && e.exe != name_or_exe);
        self.deny.retain(|d| d != name_or_exe && !exes.contains(d));
        before - (self.allow.len() + self.deny.len())
    }

    pub fn deny_exe(&mut self, exe: &str) {
        self.allow.retain(|e| e.exe != exe);
        if !self.deny.iter().any(|d| d == exe) {
            self.deny.push(exe.to_owned());
        }
    }
}

/// Acceso al registro de dpkg (`/var/lib/dpkg/info`), inyectable para los tests.
pub struct Dpkg {
    pub info_dir: PathBuf,
}

impl Default for Dpkg {
    fn default() -> Self {
        Dpkg { info_dir: PathBuf::from("/var/lib/dpkg/info") }
    }
}

impl Dpkg {
    fn list_files(&self, package: &str) -> Vec<String> {
        // Los .list pueden llevar sufijo de arquitectura (paquete:amd64).
        let mut out = vec![];
        if let Ok(rd) = std::fs::read_dir(&self.info_dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n == format!("{package}.list") || (n.starts_with(&format!("{package}:")) && n.ends_with(".list")) {
                    if let Ok(t) = std::fs::read_to_string(e.path()) {
                        out.extend(t.lines().map(str::to_owned));
                    }
                }
            }
        }
        out
    }

    pub fn owns(&self, package: &str, path: &str) -> bool {
        self.list_files(package).iter().any(|l| l == path)
    }

    /// Paquete dpkg que instalo `path`, si hay uno.
    pub fn package_of(&self, path: &str) -> Option<String> {
        for e in std::fs::read_dir(&self.info_dir).ok()?.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = n.strip_suffix(".list") {
                if std::fs::read_to_string(e.path()).map_or(false, |t| t.lines().any(|l| l == path)) {
                    return Some(stem.split(':').next().unwrap_or(stem).to_owned());
                }
            }
        }
        None
    }

    /// MD5 que dpkg registro para `path` en el paquete.
    fn md5sum(&self, package: &str, path: &str) -> Option<String> {
        let rel = path.trim_start_matches('/');
        let mut candidates = vec![self.info_dir.join(format!("{package}.md5sums"))];
        if let Ok(rd) = std::fs::read_dir(&self.info_dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with(&format!("{package}:")) && n.ends_with(".md5sums") {
                    candidates.push(e.path());
                }
            }
        }
        for c in candidates {
            if let Ok(t) = std::fs::read_to_string(c) {
                for line in t.lines() {
                    if let Some((sum, file)) = line.split_once("  ") {
                        if file == rel {
                            return Some(sum.to_owned());
                        }
                    }
                }
            }
        }
        None
    }

    /// El binario es exactamente el que dpkg instalo para `package` (propiedad + md5 coinciden).
    pub fn verifies(&self, package: &str, path: &str, content: &Path) -> bool {
        self.owns(package, path)
            && match (self.md5sum(package, path), md5_file(content)) {
                (Some(expected), Ok(actual)) => expected.eq_ignore_ascii_case(&actual),
                _ => false,
            }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("no se pudo abrir {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn md5_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Decide si la llamada se aprueba sin dialogo o se reenvia a GNOME.
pub fn decide(policy: &Policy, caller: &Caller, dpkg: &Dpkg) -> Decision {
    if policy.deny.iter().any(|d| d == &caller.exe) {
        return Decision::Forward { reason: format!("{} esta bloqueado en la politica", caller.exe) };
    }
    let Some(entry) = policy.allow.iter().find(|e| e.exe == caller.exe) else {
        return Decision::Forward { reason: format!("{} no esta en la allowlist", caller.exe) };
    };
    let actual = match sha256_file(&caller.content_path) {
        Ok(h) => h,
        Err(e) => return Decision::Forward { reason: format!("no se pudo leer el binario: {e}") },
    };
    if actual == entry.sha256 {
        return Decision::Approve { name: entry.name.clone(), monitor: entry.monitor.clone(), reason: "ruta y hash coinciden".into() };
    }
    if let Some(pkg) = &entry.package {
        if dpkg.verifies(pkg, &entry.exe, &caller.content_path) {
            return Decision::Approve {
                name: entry.name.clone(),
                monitor: entry.monitor.clone(),
                reason: format!("binario actualizado y verificado contra el md5sums del paquete {pkg}"),
            };
        }
        return Decision::Forward { reason: format!("el binario cambio y no coincide con el paquete {pkg}") };
    }
    Decision::Forward { reason: "el binario cambio desde que se autorizo".into() }
}

/// Construye una entrada para un ejecutable (hash actual y paquete dpkg si lo tiene).
pub fn entry_for(exe: &Path, dpkg: &Dpkg, monitor: Option<String>, name: Option<String>) -> Result<Entry> {
    let real = std::fs::canonicalize(exe).with_context(|| format!("no existe {}", exe.display()))?;
    let meta = std::fs::metadata(&real)?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        bail!("{} no es un ejecutable", real.display());
    }
    let exe_s = real.to_string_lossy().into_owned();
    Ok(Entry {
        name: name.unwrap_or_else(|| real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        sha256: sha256_file(&real)?,
        package: dpkg.package_of(&exe_s),
        exe: exe_s,
        monitor,
        added: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    })
}

/// Ejecutables de un paquete dpkg cuyo nombre coincide con el del paquete (p. ej. `rustdesk`).
pub fn package_executables(dpkg: &Dpkg, package: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for line in dpkg.list_files(package) {
        let p = PathBuf::from(&line);
        if p.file_name().map_or(true, |n| n.to_string_lossy() != package) {
            continue;
        }
        if let Ok(real) = std::fs::canonicalize(&p) {
            let is_exec = std::fs::metadata(&real).map_or(false, |m| m.is_file() && m.permissions().mode() & 0o111 != 0);
            if is_exec && !out.contains(&real) {
                out.push(real);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Env {
        dir: tempfile::TempDir,
        dpkg: Dpkg,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join("info");
        fs::create_dir_all(&info).unwrap();
        Env { dpkg: Dpkg { info_dir: info }, dir }
    }

    fn make_exe(e: &Env, rel: &str, content: &[u8]) -> PathBuf {
        let p = e.dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn caller(p: &Path) -> Caller {
        Caller { pid: 1, exe: fs::canonicalize(p).unwrap().to_string_lossy().into_owned(), content_path: p.to_path_buf() }
    }

    fn register_package(e: &Env, pkg: &str, exe: &Path, content: &[u8]) {
        let real = fs::canonicalize(exe).unwrap();
        let path = real.to_string_lossy().into_owned();
        fs::write(e.dpkg.info_dir.join(format!("{pkg}.list")), format!("{path}\n")).unwrap();
        let mut h = Md5::new();
        h.update(content);
        fs::write(e.dpkg.info_dir.join(format!("{pkg}.md5sums")), format!("{}  {}\n", hex(&h.finalize()), path.trim_start_matches('/'))).unwrap();
    }

    #[test]
    fn without_allowlist_everything_is_forwarded() {
        let e = env();
        let exe = make_exe(&e, "bin/tool", b"v1");
        assert!(matches!(decide(&Policy::default(), &caller(&exe), &e.dpkg), Decision::Forward { .. }));
    }

    #[test]
    fn allowed_binary_is_approved_and_others_are_not() {
        let e = env();
        let exe = make_exe(&e, "bin/tool", b"v1");
        let other = make_exe(&e, "bin/other", b"v1");
        let mut p = Policy::default();
        p.add(entry_for(&exe, &e.dpkg, Some("HDMI-2".into()), None).unwrap());
        match decide(&p, &caller(&exe), &e.dpkg) {
            Decision::Approve { name, monitor, .. } => assert_eq!((name.as_str(), monitor.as_deref()), ("tool", Some("HDMI-2"))),
            d => panic!("{d:?}"),
        }
        assert!(matches!(decide(&p, &caller(&other), &e.dpkg), Decision::Forward { .. }));
    }

    #[test]
    fn changed_binary_without_package_asks_again() {
        let e = env();
        let exe = make_exe(&e, "bin/tool", b"v1");
        let mut p = Policy::default();
        p.add(entry_for(&exe, &e.dpkg, None, None).unwrap());
        fs::write(&exe, b"v2 (reemplazado)").unwrap();
        match decide(&p, &caller(&exe), &e.dpkg) {
            Decision::Forward { reason } => assert!(reason.contains("cambio")),
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn package_update_is_accepted_only_if_dpkg_verifies_the_binary() {
        let e = env();
        let exe = make_exe(&e, "usr/share/app/app", b"v1");
        register_package(&e, "app", &exe, b"v1");
        let mut p = Policy::default();
        let entry = entry_for(&exe, &e.dpkg, None, None).unwrap();
        assert_eq!(entry.package.as_deref(), Some("app"));
        p.add(entry);
        // Actualizacion legitima: dpkg registra el contenido nuevo.
        fs::write(&exe, b"v2").unwrap();
        register_package(&e, "app", &exe, b"v2");
        assert!(matches!(decide(&p, &caller(&exe), &e.dpkg), Decision::Approve { .. }));
        // Reemplazo por otro binario que dpkg NO conoce: vuelve a pedir.
        fs::write(&exe, b"troyano").unwrap();
        assert!(matches!(decide(&p, &caller(&exe), &e.dpkg), Decision::Forward { .. }));
    }

    #[test]
    fn deny_wins_and_revoke_removes() {
        let e = env();
        let exe = make_exe(&e, "bin/tool", b"v1");
        let mut p = Policy::default();
        p.add(entry_for(&exe, &e.dpkg, None, None).unwrap());
        let real = caller(&exe).exe;
        p.deny.push(real.clone());
        assert!(matches!(decide(&p, &caller(&exe), &e.dpkg), Decision::Forward { .. }));
        assert_eq!(p.revoke("tool"), 2);
        assert!(p.allow.is_empty() && p.deny.is_empty());
    }

    #[test]
    fn policy_file_must_be_private_and_roundtrips() {
        let e = env();
        let exe = make_exe(&e, "bin/tool", b"v1");
        let file = e.dir.path().join("cfg").join("policy.toml");
        assert_eq!(Policy::load(&file).unwrap(), Policy::default());
        let mut p = Policy::default();
        p.add(entry_for(&exe, &e.dpkg, None, None).unwrap());
        p.save(&file).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Policy::load(&file).unwrap(), p);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Policy::load(&file).is_err(), "con permisos abiertos no se confia en la politica");
    }

    #[test]
    fn package_executables_finds_the_binary_named_like_the_package() {
        let e = env();
        let exe = make_exe(&e, "usr/share/rustdesk/rustdesk", b"x");
        make_exe(&e, "usr/share/rustdesk/lib/helper", b"y");
        let real = fs::canonicalize(&exe).unwrap().to_string_lossy().into_owned();
        let other = real.replace("rustdesk/rustdesk", "rustdesk/lib/helper");
        fs::write(e.dpkg.info_dir.join("rustdesk.list"), format!("{real}\n{other}\n/no/existe/rustdesk\n")).unwrap();
        let found = package_executables(&e.dpkg, "rustdesk");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], fs::canonicalize(&exe).unwrap());
    }
}
