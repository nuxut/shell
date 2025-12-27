#!/bin/bash
set -e

echo ":: Installing Nuxut Shell (Git)..."

# Ensure Arch Linux
[ ! -f /etc/arch-release ] && { echo "Error: Arch Linux required."; exit 1; }

# Install Git
sudo pacman -S --noconfirm --needed git

# Temporary Build Dir
TEMP_DIR=$(mktemp -d)
git clone https://github.com/nuxut/shell.git "$TEMP_DIR/shell" || exit 1
cd "$TEMP_DIR/shell"

# Install Dependencies
sudo pacman -S --noconfirm --needed  base-devel
sudo pacman -S --asdeps --noconfirm --needed socat fuzzel brightnessctl pavucontrol network-manager-applet blueman gnome-calendar hyprlock
# Check for Quickshell
if ! command -v quickshell >/dev/null; then
    AUR_HELPER=$(command -v yay || command -v paru)
    [ -z "$AUR_HELPER" ] && { echo "Error: Install quickshell manually (no yay/paru)."; exit 1; }
    $AUR_HELPER -S --asdeps --noconfirm --needed quickshell-git ttf-ubuntu-mono-nerd ttf-ubuntu-nerd
fi

# Build Package
makepkg -si --noconfirm

rm -rf "$TEMP_DIR"
echo ":: Done! Installed to /etc/xdg/quickshell/nuxut-shell/"
