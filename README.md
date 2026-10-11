# TURRON — Transparent User Remote Rendering & Output Negotiator

Backend de **xdg-desktop-portal** en Rust (zbus + tokio, sin librerías `-dev` de sistema) para GNOME en Wayland. Permite que un programa de escritorio remoto (hoy RustDesk) capture la pantalla y mande input **sin el diálogo del portal**, por ejemplo en un equipo sin monitor (con dongle HDMI) donde nadie puede aceptarlo — **pero solo si el dueño lo autorizó de forma explícita**. No toca ni recompila RustDesk, así que sobrevive a sus actualizaciones.

## Consentimiento: política explícita del dueño

TURRON **no elude el consentimiento**: lo reemplaza por una *allowlist* persistente que administra el dueño.

- Archivo: `~/.config/turron/policy.toml` (permisos 600, carpeta 700). Si el archivo tiene permisos más abiertos, **se ignora** y todo va al diálogo de GNOME.
- **Sin entrada en la allowlist → TURRON reenvía la llamada al backend de GNOME** (diálogo de siempre). Nada se autoaprueba: ni `install.sh` ni el servicio agregan entradas.
- Una entrada identifica el ejecutable por **ruta real + SHA-256** del binario (se lee `/proc/<pid>/exe`, que no se puede falsear desde afuera).
- **Actualizaciones:** si el hash cambió, solo se sigue aprobando cuando la entrada tiene paquete dpkg y el binario coincide con el `md5sums` que dpkg instaló para ese paquete (actualización legítima del paquete, p. ej. `rustdesk`). Cualquier otro cambio (binario reemplazado) vuelve al diálogo.
- Cada decisión y cada sesión se registra en el journal: `journalctl --user -u turrond` (`APROBADO sin dialogo…` / `REENVIADO a GNOME…`).
- RustDesk sigue exigiendo su propia contraseña / aceptación de la conexión entrante: TURRON solo aprueba la captura de pantalla local.

```sh
turronctl allow rustdesk            # autoriza el paquete dpkg 'rustdesk' (ruta + hash; acepta sus updates verificados)
turronctl allow /ruta/ejecutable    # o un ejecutable cualquiera; opcional: --monitor HDMI-2 --name nombre
turronctl list                      # muestra la política
turronctl deny <nombre|ruta>        # bloqueo explícito (siempre diálogo)
turronctl revoke <nombre|ruta>      # quita la autorización o el bloqueo
turronctl status                    # servicio, política y sesiones activas
turronctl cut <n|handle|all>        # corta una sesión activa
```

## Cómo funciona

- `turrond` implementa `org.freedesktop.impl.portal.ScreenCast` y `…RemoteDesktop` (más `Session`) con el nombre `org.freedesktop.impl.portal.desktop.turron`.
- En `CreateSession` identifica al llamador: la ruta del Request incluye su nombre de bus (`…/request/1_234/…` = `:1.234`); con él se pide el PID al bus. Con el ejecutable consulta la política.
- Autorizado: crea la sesión con la API privada de Mutter (la misma de gnome-remote-desktop): `RemoteDesktop.CreateSession` → `ScreenCast.CreateSession` enlazada → `RecordMonitor` (primario o el de la política; `RecordVirtual` si falla) → `Start` → señal `PipeWireStreamAdded`. Devuelve `streams` y, si se pidió persistencia, `persist_mode`/`restore_data`. El input (`Notify*`) va a la sesión de Mutter.
- No autorizado (o Mutter falla): se **reenvía tal cual** al backend de GNOME.
- El núcleo no sabe nada de RustDesk: RustDesk es una entrada más de la política.

## Instalar

```sh
./install.sh        # compila e instala en ~/.local; si falta el .portal de sistema, imprime la línea con sudo
sudo install -m644 data/turron.portal /usr/share/xdg-desktop-portal/portals/turron.portal   # una sola vez
./install.sh        # activa: escribe ~/.config/xdg-desktop-portal/portals.conf y reinicia xdg-desktop-portal --user
turronctl allow rustdesk   # decisión consciente del dueño
```

El `.portal` es un archivo nuevo (no pertenece a ningún paquete): las actualizaciones de Ubuntu o RustDesk no lo pisan. `portals.conf` repite las preferencias de `gnome-portals.conf` de Ubuntu y cambia solo ScreenCast y RemoteDesktop.

## Rollback

```sh
./uninstall.sh [--purge]   # restaura portals.conf, reinicia el portal, borra binarios/unit/servicio D-Bus (y la política con --purge)
sudo rm /usr/share/xdg-desktop-portal/portals/turron.portal   # opcional
```
También limpia restos del nombre anterior (rdbridge).

## Límites de GNOME que TURRON no esquiva

- **Con la pantalla bloqueada, Mutter no deja crear sesiones remotas** (`Session creation inhibited`). TURRON lo registra y reenvía a GNOME (que tampoco puede). Para uso sin monitor hay que mantener la sesión sin bloqueo automático (ajuste del dueño en GNOME) o conectar mientras esté desbloqueada.
- Solo el monitor primario (o el fijado en la política) por sesión.
- `ConnectToEIS` (libei) solo funciona para sesiones reenviadas a GNOME.
- Las sesiones reenviadas no propagan `Closed` si GNOME las cierra por su cuenta.

## Pruebas

`cargo test`: política (sin allowlist reenvía, con allowlist aprueba, hash cambiado vuelve a pedir, actualización de paquete verificada por dpkg, deny/revoke, permisos 600), identificación del llamador. Verificado en un GNOME 50 real: sesión sin diálogo, frames de PipeWire, input, reenvío a GNOME y `turronctl status/cut`.

## Licencia y autoría

TURRON es parte de **Leeway Open Software** y se distribuye bajo licencia MIT (ver `LICENSE`).
