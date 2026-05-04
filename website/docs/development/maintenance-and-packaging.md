# Maintenance and Packaging

This guide collects the routine commands used to keep a development checkout
healthy, preview and deploy the website, and build distributable packages.

## Daily Maintenance

Format and inspect the Rust workspace:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace
```

Run the full Rust documentation build:

```bash
cargo doc --workspace --no-deps
```

Clean all Cargo build artifacts when target output is stale or disk usage is
high:

```bash
cargo clean
```

Clean only one package when a focused reset is enough:

```bash
cargo clean -p y-web
cargo clean -p y-cli
```

The full clean removes the whole `target` directory and makes the next build
expensive. Prefer package-level cleanups during normal development.

Refresh frontend dependencies and run the shared frontend gate:

```bash
cd crates/y-gui
npm install
npm test
npm run lint
npm run build
npm run build:web
```

Run service health checks against a local stack:

```bash
AGENT_URL=http://127.0.0.1:3000 ./scripts/health-check.sh --wait 30
```

`health-check.sh` also checks Qdrant through `QDRANT_URL`, which defaults to
`http://localhost:6333`.

## Website Preview

The website is a VitePress site in `website`.

Install dependencies:

```bash
cd website
pnpm install --frozen-lockfile
```

Start local preview while editing docs:

```bash
cd website
pnpm run dev
```

Build the static site:

```bash
cd website
pnpm run build
```

Preview the built output locally:

```bash
cd website
pnpm run preview
```

CI uses `pnpm install --frozen-lockfile` and `pnpm run build`, so prefer pnpm
for website work even though the package scripts are standard npm scripts.

## Website Deployment

Website deployment is handled by `scripts/deploy-website.sh`, which builds
`website/docs/.vitepress/dist` and deploys it to Cloudflare Pages with
Wrangler.

Prerequisites:

- `pnpm`
- `wrangler`
- Cloudflare Pages project access

Deploy production:

```bash
./scripts/deploy-website.sh
```

Deploy a preview:

```bash
./scripts/deploy-website.sh --preview
```

Deploy an existing build without rebuilding:

```bash
./scripts/deploy-website.sh --no-build
```

Useful environment overrides:

```bash
CF_PROJECT=y-agent CF_BRANCH=main ./scripts/deploy-website.sh
```

## Native Installation

For a local host install without Docker, use `scripts/native-install.sh`.

Install a release build to `/usr/local/bin`:

```bash
./scripts/native-install.sh --release
```

Install a debug build to a custom prefix:

```bash
./scripts/native-install.sh --debug --prefix "$HOME/.local"
```

Use an existing binary:

```bash
./scripts/native-install.sh --skip-build --prefix "$HOME/.local"
```

The script creates user data and config directories and copies example config
files when they are missing.

## Release Package Builds

`scripts/build-release.sh` is the main packaging entry point. It writes zip
archives to `dist`.

Build both CLI and GUI packages for the current platform:

```bash
./scripts/build-release.sh
```

Build only the CLI package:

```bash
./scripts/build-release.sh cli
```

Build only the GUI package:

```bash
./scripts/build-release.sh gui
```

Override the version string used in archive names:

```bash
./scripts/build-release.sh --version 0.6.2
```

Cross-compile with an installed Rust target:

```bash
rustup target add aarch64-apple-darwin
./scripts/build-release.sh --target aarch64-apple-darwin
```

The script validates common cross-compilation constraints. Windows MSVC targets
must be built on Windows. Windows GNU targets require a MinGW-w64 toolchain and
GUI bundles require NSIS.

## Platform Package Outputs

The release script collects platform-native GUI bundles into the GUI zip:

| Platform | Expected GUI artifacts |
|----------|------------------------|
| macOS | `.dmg` |
| Linux | `.deb`, `.AppImage`, optional `.pkg.tar.zst` |
| Windows | `.msi`, `.exe` |

Linux AppImage patching can also be run manually:

```bash
./scripts/package-linux-appimage.sh \
  --source-appimage target/release/bundle/appimage/y-agent.AppImage \
  --output-dir dist/appimage
```

Build an Arch Linux pacman package manually:

```bash
./scripts/package-linux-pacman.sh \
  --version 0.6.2 \
  --binary-path target/release/y-gui \
  --output-dir dist/pacman
```

`package-linux-pacman.sh` requires `makepkg`.

## Version Bumps

Use the version bump script to keep the workspace version files aligned:

```bash
./scripts/bump-version.sh 0.6.2
```

The script updates the workspace Cargo version, the GUI package version, Tauri
metadata, and `package.nix`.

## Final Verification

For a mixed Rust, GUI, Web API, and docs change, run:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps

cd crates/y-gui
npm test
npm run lint
npm run build
npm run build:web

cd ../../website
pnpm run build
```
