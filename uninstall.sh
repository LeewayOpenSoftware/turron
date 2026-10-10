#!/bin/sh
# Deshace install.sh por completo (rollback). El archivo de sistema se quita aparte, con sudo.
set -u
CONF="$HOME/.config/xdg-desktop-portal/portals.conf"
MARK="# rdbridge: gestionado por install.sh"

if [ -f "$CONF" ] && grep -q "$MARK" "$CONF"; then
    rm -f "$CONF"
    if [ -f "$CONF.rdbridge-backup" ]; then
        mv "$CONF.rdbridge-backup" "$CONF"
        echo "restaurado $CONF desde la copia de seguridad"
    fi
    systemctl --user restart xdg-desktop-portal.service 2>/dev/null || true
    echo "portal restaurado a la configuracion de GNOME"
fi
systemctl --user stop rdbridge.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/rdbridge.service" \
      "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.rdbridge.service" \
      "$HOME/.local/bin/rdbridge"
systemctl --user daemon-reload 2>/dev/null || true
echo "rdbridge desinstalado (usuario)."
if [ -f /usr/share/xdg-desktop-portal/portals/rdbridge.portal ]; then
    echo "Opcional, con sudo: sudo rm /usr/share/xdg-desktop-portal/portals/rdbridge.portal"
fi
