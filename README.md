# RDBridge

Backend de **xdg-desktop-portal** en Rust (zbus + tokio, sin librerías `-dev` de sistema) para GNOME en Wayland. Sirve para que **RustDesk** capture la pantalla y mande input **sin el diálogo del portal**, por ejemplo en un equipo sin monitor (con dongle HDMI) donde nadie puede aceptar el cuadro. **No toca ni recompila RustDesk**, así que sobrevive a sus actualizaciones.

## Cómo funciona

- Implementa `org.freedesktop.impl.portal.ScreenCast` y `org.freedesktop.impl.portal.RemoteDesktop` (más el objeto `Session`) con el nombre `org.freedesktop.impl.portal.desktop.rdbridge`.
- En `CreateSession` identifica al llamador: la ruta del Request del portal contiene su nombre de bus (`.../request/1_234/...` = `:1.234`); con él pide el PID al bus y lee `/proc/<pid>/exe`.
  - Si el ejecutable está en `/usr/share/rustdesk/`, `/usr/lib/rustdesk/` o es `/usr/bin/rustdesk`, la sesión se crea con la API privada de Mutter (la misma que usa gnome-remote-desktop): `RemoteDesktop.CreateSession` → `ScreenCast.CreateSession` enlazada → `RecordMonitor("")` (monitor primario; `RecordVirtual` si falla) → `Start` → señal `PipeWireStreamAdded` con el nodo. Devuelve `streams` y, si se pidió persistencia, `persist_mode` y `restore_data`. El input (`Notify*`) va a la sesión de Mutter.
  - Cualquier otro llamador se **reenvía tal cual** al backend de GNOME (`org.freedesktop.impl.portal.desktop.gnome`), con su diálogo de siempre.
- Si Mutter falla al crear la sesión, también se reenvía a GNOME en vez de dejar al llamador sin respuesta.

## Instalar

```sh
./install.sh        # compila e instala en ~/.local; si falta el .portal de sistema, imprime la línea con sudo
sudo install -m644 data/rdbridge.portal /usr/share/xdg-desktop-portal/portals/rdbridge.portal   # una sola vez
./install.sh        # activa: escribe ~/.config/xdg-desktop-portal/portals.conf y reinicia xdg-desktop-portal --user
```

El archivo `.portal` es nuevo (no pertenece a ningún paquete), así que las actualizaciones de Ubuntu o de RustDesk no lo pisan. `portals.conf` repite las preferencias de `gnome-portals.conf` de Ubuntu y cambia solo ScreenCast y RemoteDesktop.

## Rollback

```sh
./uninstall.sh      # restaura portals.conf, reinicia el portal y borra binario, unit y servicio D-Bus
sudo rm /usr/share/xdg-desktop-portal/portals/rdbridge.portal   # opcional
```

## Pruebas

- `cargo test`: identificación del llamador y mapeo de cursor.
- `RDBRIDGE_ALLOW=/ruta/exacta/a/otro/ejecutable` agrega ejecutables aprobados (solo pruebas); `RDBRIDGE_LOG=rdbridge=debug` sube el log.
- Con el servicio en marcha se puede llamar al backend directamente (sin pasar por el portal) construyendo rutas `/org/freedesktop/portal/desktop/request/<tu nombre de bus>/<token>`: así se verificó la secuencia completa y los frames de PipeWire.

## Limitaciones

- Solo el monitor primario (RustDesk con varios monitores verá uno).
- `ConnectToEIS` (libei) solo funciona para sesiones reenviadas a GNOME.
- Las sesiones reenviadas no propagan la señal `Closed` si GNOME cierra la sesión por su cuenta.
