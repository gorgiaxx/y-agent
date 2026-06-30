# drozer Usage Guide

## Purpose
drozer is an Android security testing framework. It acts as an installed app to interact with the Android Runtime, IPC endpoints of other apps, and the underlying OS — used to discover security vulnerabilities in apps and devices.

## Architecture
Dual-endpoint architecture: **PC Console + Device Agent**.
- **Console (PC)**: Command-line environment; runs modules to assess the device.
- **Agent (device APK)**: Embeds a server; accepts Console connections; executes operations as an app.
- **Protocol**: Custom binary protocol `drozerp`, default port **31415**.
- **Reflection**: Console remotely invokes Java classes on the device via a Reflector to perform app-level operations.

## Install & Connect
1. Install Console: `pipx install drozer` (requires Python 3.8+, JDK 11+).
2. Install Agent: `adb install drozer-agent.apk`, open Agent → "Embedded Server" → "Enable".
3. Establish a session:
   - Network: `drozer console connect --server <device_IP>`
   - USB (first `adb forward tcp:31415 tcp:31415`): `drozer console connect`
4. The `dz>` prompt means connected (shows device Android ID, manufacturer, model, OS version).

## Top-level Commands (`drozer [COMMAND]`)
- `console` — Start Console (core interactive entry point)
- `server` — Start Server (infrastructure mode: session routing / resource hosting)
- `exploit` — Generate exploits (`exploit build/info/list`)
- `payload` — Generate payloads (`payload build/info/list`)
- `agent` — Build a custom Agent (BETA, currently crashes)
- `module` — Manage modules (install/search/repository management)
- `ssl` — Manage SSL key material

## Console Built-in Commands (at `dz>` prompt)
- `run MODULE` — Execute a module (core command)
- `list` (aliases `l`/`ls`/`ll`) — List executable modules (hides unauthorized ones)
- `shell` — Start an interactive Linux shell on the device (Agent process context)
- `cd` — Mount a namespace as session root; shortens module fully-qualified names
- `clean` — Remove drozer temporary files on the device
- `help [COMMAND/MODULE]` — Show help
- `load FILE` — Load and sequentially run commands from a file
- `module install NAME` — Install extra modules from the network
- `permissions` — Show Agent granted permissions
- `set`/`unset` — Set/unset environment variables passed to shell
- `exit` — Terminate the session

## Module System
All functionality is delivered as "modules", invoked via `run <fully-qualified-name>`.

**Naming convention**: module name = `path` list + lowercased class name. e.g. `path=["app","package"]` + class `AttackSurface` → `app.package.attacksurface`. Use `cd` to shorten a namespace.

**Module authoring essentials** (subclass `drozer.modules.Module`):
- Class attributes: `name`, `description`, `examples`, `author`, `date`, `license`, `path` (namespace path), `permissions` (required Agent permissions), `module_type` (default `"drozer"`; exploit modules use `"exploit"`).
- `add_arguments(parser)` — Add CLI args via argparse.
- `execute(arguments)` — Execution logic entry point.
- Multiple-inherit `common.*` mixins for capabilities: `PackageManager`, `Filters`, `Provider`, `Shell`, `FileSystem`, `Assets`, `ServiceBinding`, `Vulnerability`/`VulnerabilityScanner`, `Exploit`, `Loader` (ClassLoader), etc.
- Resolve Java classes via `self.klass(class_name)`, instantiate with `self.new(...)`, output via `self.stdout.write()`, reflect via `self.reflector`.

**Permission mechanism**: the `permissions` attribute determines whether `list` shows the module; modules for which the Agent lacks permissions are hidden.

## Sub-document Index
- **Module Reference** (`details/module-reference.md`): Complete module list grouped by namespace. Load when you need to find a specific module or confirm a fully-qualified name.
- **Assessment Workflow & Advanced** (`details/assessment-workflow.md`): Typical assessment steps, infrastructure mode, important conventions. Load when performing actual testing or handling complex connections.
