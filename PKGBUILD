# Maintainer: Hakan Göksu <hakan@goksu.me>
pkgname=nuxut-shell-git
pkgver=1.0.0
pkgrel=1
pkgdesc="A modern, unified desktop shell built with Quickshell for Nuxut (Git version)."
arch=('x86_64')
url="https://github.com/nuxut/shell"
license=('MIT')
depends=('quickshell' 'ttf-font-nerd' 'socat' 'fuzzel' 'brightnessctl' 'pavucontrol' 'network-manager-applet' 'blueman' 'gnome-calendar' 'hyprlock')
provides=('nuxut-shell')
conflicts=('nuxut-shell')
source=("git+https://github.com/nuxut/shell.git")
md5sums=('SKIP')

pkgver() {
    cd "$srcdir/shell"
    printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

package() {
    install -d "$pkgdir/etc/xdg/quickshell/nuxut-shell"

    cp -r "$srcdir/shell/"* "$pkgdir/etc/xdg/quickshell/nuxut-shell/"
    
    rm -f "$pkgdir/etc/xdg/quickshell/nuxut-shell/PKGBUILD"
    rm -f "$pkgdir/etc/xdg/quickshell/nuxut-shell/setup.sh"
    rm -rf "$pkgdir/etc/xdg/quickshell/nuxut-shell/.git"
    rm -f "$pkgdir/etc/xdg/quickshell/nuxut-shell/.gitignore"
    
    chmod -R 644 "$pkgdir/etc/xdg/quickshell/nuxut-shell/"*
    find "$pkgdir/etc/xdg/quickshell/nuxut-shell/" -type d -exec chmod 755 {} +
}
