# Download

<DownloadHero />

## System Requirements

| Dependency | Required? | Notes |
|------------|-----------|-------|
| **Rust 1.94+** | Yes | Pinned in `rust-toolchain.toml` |
| **Node.js 18+** | GUI only | For building the Tauri desktop app |
| **SQLite 3.35+** | Embedded | Bundled, no action needed |
| **Chrome / Chromium** | Optional | For the browser tool (auto-detected) |
| Qdrant | Optional | For semantic vector search (knowledge base, memory) |

## Build from Source

### CLI + Web Server

```bash
git clone https://github.com/gorgias/y-agent.git
cd y-agent

cargo build --release
# Binary: target/release/y-agent
```

### GUI Desktop App (Tauri v2)

```bash
cd crates/y-gui && npm install && cd ../..
./scripts/build-release.sh gui
# Output: dist/y-agent-gui-<version>-<platform>.zip
#   macOS:   .pkg
#   Linux:   .deb, .AppImage, .pkg.tar.zst
#   Windows: .msi, .exe
```

### Full Release Build

```bash
./scripts/build-release.sh
# Builds both CLI zip and GUI bundle
```

### Nix

```bash
nix build           # Build the CLI package
nix develop          # Enter dev shell with all dependencies
```

## Installation

### macOS

1. Download the `.pkg` installer from [GitHub Releases](https://github.com/gorgias/y-agent/releases).
2. Run the installer. It installs `y-agent.app` in Applications and the CLI as
   `/usr/local/bin/yagent`.
3. Verify the command with `yagent --help`. The compatibility name `y-agent`
   is also installed.
4. On first launch, allow the app in System Settings > Privacy & Security if
   macOS requests confirmation.

Use the `.pkg` for upgrades as well as first-time installation. It can replace
an administrator-owned application bundle and updates the command-line tools;
dragging an `.app` over such an installation in Finder can fail with a locked
or read-only items message.

Use `yagent` as the canonical command installed by the package. If the
compatibility command `y-agent` reports an older version immediately after
installation, run `rehash` in zsh or open a new terminal. Existing shells can
cache an earlier Cargo installation under `~/.cargo/bin`; use `which -a y-agent`
to compare it with `/usr/local/bin/y-agent`.

### Linux

```bash
# Debian/Ubuntu
sudo dpkg -i y-agent_<version>_amd64.deb

# Arch Linux / pacman
sudo pacman -U y-agent-gui-<version>-1-x86_64.pkg.tar.zst

# AppImage
chmod +x y-agent_<version>_amd64.AppImage
./y-agent_<version>_amd64.AppImage
```

### Windows

1. Download the `.msi` installer from [GitHub Releases](https://github.com/gorgias/y-agent/releases)
2. Run the installer and follow the wizard
3. Launch from the Start Menu
