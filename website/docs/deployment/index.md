# Deployment

## Docker Compose

The included `docker-compose.yml` provisions three services:

- **y-agent** -- Main application (port 8080)
- **PostgreSQL 16** -- Diagnostics & analytics
- **Qdrant v1.8.4** -- Vector store for knowledge base & memory

```bash
y-agent init
docker compose up -d
./scripts/health-check.sh
docker compose logs -f y-agent
```

## Native Install

```bash
./scripts/native-install.sh
# Or customize:
./scripts/native-install.sh --prefix ~/.local --data-dir ~/y-agent-data
```

Creates:
- Binary at `$PREFIX/bin/y-agent`
- Config at `~/.config/y-agent/`
- Data at `~/.local/share/y-agent/`

## CI/CD (GitHub Actions)

Push a version tag to trigger the CI/CD pipeline:

```bash
./scripts/bump-version.sh 0.2.0    # Update version across Cargo.toml, package.json, etc.
git tag v0.2.0 && git push origin v0.2.0
```

The CI pipeline (`.github/workflows/ci.yml`) runs:

1. **Format** -- `cargo fmt --check`
2. **Build & Test** -- clippy, check, test, doc (single runner with shared compilation cache)

### Required GitHub Secrets

| Secret | Description |
|--------|-------------|
| `DEPLOY_HOST` | Target server address |
| `DEPLOY_USER` | SSH username |
| `DEPLOY_SSH_KEY` | SSH private key |
| `DEPLOY_PATH` | Deployment directory on server |

## Quality Gates

After any code change, all checks must pass:

```bash
cargo fmt --all
cargo clippy --fix --allow-dirty --workspace -- -D warnings
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps
```
