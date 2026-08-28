# Isolated JavaScript Launch Hook

This schema-v5 payload is loaded by `dev.hmclce.runtime.quickjs-host`. It validates the frozen Aura context, observes `before-game-launch`, and returns the ordered launch-plan Map without changing its values.

Build the deterministic example package from the repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/package-javascript-plugin.ps1 `
  -Source examples/launch-hook `
  -Output artifacts/dev.hmclce.example.javascript.launch-hook-v1.0.0.npl
```

The JVM capability token never enters JavaScript. Bridge calls are reauthorized by Aura against the original payload context.
