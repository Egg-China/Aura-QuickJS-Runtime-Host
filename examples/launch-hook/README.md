# Isolated JavaScript Hook and Patch

This schema-v5 payload requires a build of the current `dev.hmclce.runtime.quickjs-host` source and
the pinned Aura build described in the [repository README](../../README.md). The already published
beta downloads have not been updated by this source change.

The payload validates the frozen Aura context and observes two callbacks without side effects:

- `hook.before-game-launch` returns the ordered Map `contractVersion: 1n`, `action: "unchanged"`.
- `aura.patch.v1` observes an `after` Patch on
  `org.jackhuang.hmcl.util.io.FileUtils.getName(java.nio.file.Path)` and returns the ordered Map
  `schemaVersion: 1n`, `action: "unchanged"`.

The manifest declares `launcher-hook` and `launcher-patch` in both permission lists. Aura must grant
these permissions; loading a package alone is not authorization. The sample never modifies the
request or keeps invocation-local handles. Version fields are `bigint`, not JavaScript `number`,
and the ordered result is a `Map`, not a plain object or the original request.

Build the deterministic example package from the repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/package-javascript-plugin.ps1 `
  -Source examples/launch-hook `
  -Output artifacts/dev.hmclce.example.javascript.launch-hook-v1.0.0.npl
```

The JVM capability token never enters JavaScript. Bridge calls are reauthorized by Aura against the original payload context.

Hook invocation uses the dispatcher's original timeout and callback ID `0`. Patch callbacks also
use callback ID `0`; Aura owns authorization, invocation-local handle lifetime, and failure fallback.
Neither callback can extend a handle beyond its invocation. Payload logs must not go to stdout,
which is reserved for process protocol frames.

`cargo test -p aura-quickjs-host --test process_callbacks` runs the actual `--stdio` executable,
checks lifecycle IDs and literal Hook/Patch wire vectors, and kills/reaps a child that exceeds the
test deadline. Java Provider tests additionally execute the example through Aura's real process SPI.
