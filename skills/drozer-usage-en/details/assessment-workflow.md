# drozer Assessment Workflow & Advanced

## Typical Security Assessment Workflow
Complete assessment steps using a vulnerable test app (e.g. Sieve):

1. **Locate the app**: `run app.package.list -f <keyword>`
2. **View package info**: `run app.package.info -a <package>`
3. **Identify attack surface**: `run app.package.attacksurface <package>` (lists exported activity/receiver/provider/service and debuggable flag)
4. **View & start exported Activities**:
   - `run app.activity.info -a <package>`
   - Bypass authorization screen: `run app.activity.start --component <package> <component>`
5. **View Content Providers**: `run app.provider.info -a <package>`
6. **Discover accessible URIs**: `run scanner.provider.finduris -a <package>`
7. **Query/read Provider data**:
   - `run app.provider.query <content_uri> --vertical`
   - File-type Provider: `run app.provider.read <content_uri>`
   - Download database: `run app.provider.download <content_uri> <local_path>`
8. **SQL injection testing**:
   - Manual probing: `run app.provider.query <uri> --projection "'"` or `--selection "'"`
   - Automated scan: `run scanner.provider.injection -a <package>`
9. **Interact with Services**: `run app.service.info -a <package>` → `run app.service.send`
10. **File transfer & device shell**: `run tools.file.upload/download`; `shell`

## Infrastructure Mode
Used when the device IP is unknown or NAT/firewall traversal is needed:

1. Start Server on a reachable machine: `drozer server start`
2. Agent side — add an Endpoint: Settings → New Endpoint → enter Host/Port
3. Console side:
   - List devices: `drozer console devices --server <server>:31415`
   - Connect to a device: `drozer console connect <device_id> --server <server>:31415`

The Server simultaneously acts as drozerp / http / bytestream / bind shell server, hosting resources for exploits and receiving call-backs from compromised devices.

## Important Conventions & Notes
- **Module discovery**: At runtime, all `Module` subclasses are reflected and registered by fully-qualified name; name conflicts are handled by `ImportConflictResolver`.
- **Module load paths**: Built-in `src/drozer/modules` + user repository paths in config (managed via `drozer-repository`).
- **Exploit/payload modules**: `module_type` is not `"drozer"`; loaded by the `exploit`/`payload` top-level commands respectively, not shown in `list`.
- **Reflection return values**: Reflection calls return `ReflectedType`; use `.native()` to convert to Python values, `self.arg(value, obj_type)` to force a Java type.
- **Output convention**: All output goes through `self.stdout.write()` to support colored/uncolored streams.
- **Platform limitation**: Not natively supported on Windows; use Docker or WSL.
- **Official module repository**: Currently unavailable (under maintenance); custom modules must be installed manually.
