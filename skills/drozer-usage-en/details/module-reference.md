# drozer Module Reference

drozer module list grouped by namespace. All modules are invoked via `run <fully-qualified-name>`; use `cd` to shorten a namespace.

## app.* — App IPC component operations

**Package management (app.package.*)**
- `app.package.list` — List installed packages
- `app.package.info` — View package details
- `app.package.manifest` — View AndroidManifest
- `app.package.launchintent` — View launch Intent
- `app.package.attacksurface` — List exported components and debuggable flag
- `app.package.native` / `app.package.shareduid` / `app.package.debuggable` / `app.package.backup`

**Activity (app.activity.*)**
- `app.activity.info` — View exported Activities
- `app.activity.start` — Start an Activity (can bypass authorization)
- `app.activity.forintent` — Find Activities that handle a given Intent

**Service (app.service.*)**
- `app.service.info` / `app.service.start` / `app.service.stop` / `app.service.send`

**Broadcast (app.broadcast.*)**
- `app.broadcast.info` / `app.broadcast.send` / `app.broadcast.sniff`

**Content Provider (app.provider.*)**
- `app.provider.info` — View Provider info
- `app.provider.query` — Query Provider data
- `app.provider.columns` — View column names
- `app.provider.insert` / `app.provider.update` / `app.provider.delete` — CRUD operations
- `app.provider.read` — Read file-type Providers
- `app.provider.download` — Download a database file
- `app.provider.call` — Invoke a Provider method
- `app.provider.finduri` — Find Content URIs

## scanner.* — Automated vulnerability scanning

**Provider scanning (scanner.provider.*)**
- `scanner.provider.finduris` — Discover accessible content URIs
- `scanner.provider.injection` — SQL injection detection
- `scanner.provider.traversal` — Directory traversal detection
- `scanner.provider.sqltables` — Dump SQL tables

**Other scanning (scanner.misc.* / scanner.activity.*)**
- `scanner.activity.browsable` — Browsable Activity detection
- `scanner.misc.readable_files` / `scanner.misc.writable_files` — Readable/writable file detection
- `scanner.misc.sflag_binaries` — SUID binary detection
- `scanner.misc.secretcodes` — Dial secret-code detection
- `scanner.misc.native` — Native library detection

## information.* — Device/permission info
- `information.device_info` / `information.datetime` / `information.permissions`

## shell.* — Device shell
- `shell.start` — Start interactive shell
- `shell.exec` — Execute a single command
- `shell.send` — Send data to a shell

## tools.* — Files & utilities
- `tools.file.upload` / `tools.file.download` — Upload/download files
- `tools.file.size` / `tools.file.md5sum` — File size/checksum
- `tools.setup.toybox` / `tools.setup.minimalsu` — Install toolsets

## auxiliary.* — Auxiliary
- `auxiliary.web_content_resolver` / `auxiliary.handler`

## exploit.* — Exploit modules
`module_type="exploit"`; generated via `drozer exploit build <name> --payload <payload> --server <host>`.
- Namespace examples: `exploit.remote.webview.addjavascriptinterface`, `exploit.remote.browser.*`, `exploit.jdwp.check`, `exploit.pilfer.*`, `exploit.soceng.*`, `exploit.dos.remote_wipe`, `exploit.fileformat.*`

## payloads — Payload modules
Generated via `drozer payload build <name>`.
- `weasel.shell.armeabi` — weasel agent (full-featured post-exploitation agent)
- `shell.reverse_tcp_shell` — Reverse shell
- `shell.reverse_weasel` — Reverse weasel
