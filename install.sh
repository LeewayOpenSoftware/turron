#!/bin/sh
# Instala TURRON para el usuario actual (sin sudo) y lo activa si el archivo .portal de
# sistema ya existe. Se puede correr varias veces. Rollback total: ./uninstall.sh
#
# IMPORTANTE: este script NO autoriza a ningun programa. Mientras la allowlist este vacia,
# TURRON reenvia todo al dialogo normal de GNOME. El dueño autoriza con `turronctl allow ...`.
set -eu
cd "$(dirname "$0")"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

PORTAL_FILE=/usr/share/xdg-desktop-portal/portals/turron.portal
CONF_DIR="$HOME/.config/xdg-desktop-portal"
CONF="$CONF_DIR/portals.conf"
MARK="# turron: gestionado por install.sh"

echo "== compilando"
cargo build --release
echo "== instalando archivos de usuario"
install -Dm755 target/release/turrond "$HOME/.local/bin/turrond"
install -Dm755 target/release/turronctl "$HOME/.local/bin/turronctl"
mkdir -p "$HOME/.config/systemd/user" "$HOME/.local/share/dbus-1/services"
sed "s|@HOME@|$HOME|g" data/turrond.service > "$HOME/.config/systemd/user/turrond.service"
sed "s|@HOME@|$HOME|g" data/org.freedesktop.impl.portal.desktop.turron.service > "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.turron.service"
systemctl --user daemon-reload

if [ ! -f "$PORTAL_FILE" ]; then
    echo
    echo "Falta UN archivo de sistema (requiere sudo). Ejecutar:"
    echo "  sudo install -m644 $(pwd)/data/turron.portal $PORTAL_FILE"
    echo "y despues volver a correr ./install.sh para activar TURRON."
    exit 0
fi

echo "== activando (portals.conf de usuario)"
mkdir -p "$CONF_DIR"
if [ -f "$CONF" ] && ! grep -q "$MARK" "$CONF"; then
    cp -n "$CONF" "$CONF.turron-backup"
    echo "   copia de seguridad: $CONF.turron-backup"
fi
# Mismas preferencias que gnome-portals.conf de Ubuntu, mas ScreenCast/RemoteDesktop -> turron.
cat > "$CONF" <<CONFEOF
$MARK
[preferred]
default=gnome;gtk;
org.freedesktop.impl.portal.Secret=gnome-keyring;
org.freedesktop.impl.portal.ScreenCast=turron;
org.freedesktop.impl.portal.RemoteDesktop=turron;
CONFEOF
systemctl --user restart xdg-desktop-portal.service
echo
echo "TURRON activo. Politica actual:"
"$HOME/.local/bin/turronctl" list
echo "Para autorizar un programa (decision del dueño):  turronctl allow rustdesk"
