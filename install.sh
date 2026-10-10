#!/bin/sh
# Instala rdbridge para el usuario actual (sin sudo) y lo activa si el archivo .portal de
# sistema ya existe. Se puede correr varias veces. Rollback total: ./uninstall.sh
set -eu
cd "$(dirname "$0")"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

PORTAL_FILE=/usr/share/xdg-desktop-portal/portals/rdbridge.portal
CONF_DIR="$HOME/.config/xdg-desktop-portal"
CONF="$CONF_DIR/portals.conf"
MARK="# rdbridge: gestionado por install.sh"

echo "== compilando"
cargo build --release
echo "== instalando archivos de usuario"
install -Dm755 target/release/rdbridge "$HOME/.local/bin/rdbridge"
mkdir -p "$HOME/.config/systemd/user" "$HOME/.local/share/dbus-1/services"
sed "s|@HOME@|$HOME|g" data/rdbridge.service > "$HOME/.config/systemd/user/rdbridge.service"
sed "s|@HOME@|$HOME|g" data/org.freedesktop.impl.portal.desktop.rdbridge.service > "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.rdbridge.service"
systemctl --user daemon-reload

if [ ! -f "$PORTAL_FILE" ]; then
    echo
    echo "Falta UN archivo de sistema (requiere sudo). Ejecutar:"
    echo "  sudo install -m644 $(pwd)/data/rdbridge.portal $PORTAL_FILE"
    echo "y despues volver a correr ./install.sh para activar rdbridge."
    exit 0
fi

echo "== activando (portals.conf de usuario)"
mkdir -p "$CONF_DIR"
if [ -f "$CONF" ] && ! grep -q "$MARK" "$CONF"; then
    cp -n "$CONF" "$CONF.rdbridge-backup"
    echo "   copia de seguridad: $CONF.rdbridge-backup"
fi
# Mismas preferencias que gnome-portals.conf de Ubuntu, mas ScreenCast/RemoteDesktop -> rdbridge.
cat > "$CONF" <<CONFEOF
$MARK
[preferred]
default=gnome;gtk;
org.freedesktop.impl.portal.Secret=gnome-keyring;
org.freedesktop.impl.portal.ScreenCast=rdbridge;
org.freedesktop.impl.portal.RemoteDesktop=rdbridge;
CONFEOF
systemctl --user restart xdg-desktop-portal.service
echo "rdbridge activo. Ver log: journalctl --user -u rdbridge -f"
