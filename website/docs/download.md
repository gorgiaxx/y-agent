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
#   macOS:   .dmg, .app
#   Linux:   .deb, .AppImage
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

1. Download the `.dmg` from [GitHub Releases](https://github.com/gorgias/y-agent/releases)
2. Open the `.dmg` and drag y-agent to the Applications folder
3. On first launch, allow it in System Settings > Privacy & Security

### Linux

```bash
# Debian/Ubuntu
sudo dpkg -i y-agent_<version>_amd64.deb

# AppImage
chmod +x y-agent_<version>_amd64.AppImage
./y-agent_<version>_amd64.AppImage
```

### Windows

1. Download the `.msi` installer from [GitHub Releases](https://github.com/gorgias/y-agent/releases)
2. Run the installer and follow the wizard
3. Launch from the Start Menu
