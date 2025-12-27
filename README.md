# Nuxut Shell

A modern, unified desktop shell built with [Quickshell](https://github.com/quickshell-mirror/quickshell) for the Nuxut ecosystem.

## Installation

### Single Command (Recommended)
Installs dependencies, builds the package, and sets up configuration:
```bash
curl -s https://raw.githubusercontent.com/nuxut/shell/main/setup.sh | bash
```

### Manual Installation
1. Install dependencies: `quickshell` (AUR), `ttf-ubuntu-mono-nerd`.
2. Clone the repo and build:
```bash
git clone https://github.com/nuxut/shell.git
cd shell
makepkg -si
```

## Configuration

The configuration file is located at:
`~/.config/nuxut/quickshell/config.json`

It is automatically generated on first run with the **Catppuccin Frappe Sapphire** theme.

**Default Palette:**
- Bar: `#303446` (Base)
- Accent: `#85c1dc` (Sapphire)
- Text: `#c6d0f5` (Text)
