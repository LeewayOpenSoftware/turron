#!/bin/sh
# Deshace install.sh por completo (rollback). Tambien limpia restos del nombre anterior
# (rdbridge). La politica del dueño (~/.config/turron) se conserva salvo que se pida --purge.
# El archivo de sistema se quita aparte, con sudo.
set -u
CONF="$HOME/.config/xdg-desktop-portal/portals.conf"
changed=0

for MARK in "# turron: gestionado por install.sh" "# rdbridge: gestionado por install.sh"; do
    if [ -f "$CONF" ] && grep -q "$MARK" "$CONF"; then
        rm -f "$CONF"
        changed=1
        for B in "$CONF.turron-backup" "$CONF.rdbridge-backup"; do
            if [ -f "$B" ]; then
                mv "$B" "$CONF"
                echo "restaurado $CONF desde $B"
            fi
        done
    fi
done
if [ "$changed" = 1 ]; then
    systemctl --user restart xdg-desktop-portal.service 2>/dev/null || true
    echo "portal restaurado a la configuracion de GNOME"
fi

for U in turrond rdbridge; do
    systemctl --user stop "$U.service" 2>/dev/null || true
    rm -f "$HOME/.config/systemd/user/$U.service"
done
rm -f "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.turron.service" \
      "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.rdbridge.service" \
      "$HOME/.local/bin/turrond" "$HOME/.local/bin/turronctl" "$HOME/.local/bin/rdbridge"
systemctl --user daemon-reload 2>/dev/null || true

if [ "${1:-}" = "--purge" ]; then
    rm -rf "$HOME/.config/turron"
    echo "politica borrada"
else
    echo "(la politica en ~/.config/turron se conserva; usar --purge para borrarla)"
fi
echo "TURRON desinstalado (usuario)."
for P in turron rdbridge; do
    if [ -f "/usr/share/xdg-desktop-portal/portals/$P.portal" ]; then
        echo "Opcional, con sudo: sudo rm /usr/share/xdg-desktop-portal/portals/$P.portal"
    fi
done
