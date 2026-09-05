# Aura QuickJS Author SDK

JavaScript payloads are UTF-8 ES modules described by `aura-javascript.json`. They import the frozen Bridge API from `aura:runtime`, target runtime ABI 1, and run only in an isolated Host process.

Use `aura-runtime.d.ts` for `AuraValue`, lifecycle context, Bridge, handle, and stable error types. The first beta ships these declarations as a Release archive and does not publish an npm package.

The [current Hook/Patch example](../examples/launch-hook/README.md) uses ordered `Map` responses
with `bigint` contract versions. It requires the current Host source and pinned Aura build;
already published beta archives are unchanged. Invocation-local Patch handles must not be retained
or stored for later callbacks.

Validate types and package a payload from the repository root:

```powershell
npm --prefix sdk ci --ignore-scripts
npm --prefix sdk exec tsc -- --project sdk/tsconfig.json --noEmit
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/package-javascript-plugin.ps1 `
  -Source examples/launch-hook `
  -Output artifacts/launch-hook.npl
```
