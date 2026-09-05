# Aura QuickJS Runtime Host

Aura QuickJS Runtime Host is an optional schema-v5 Runtime Provider for Aura Launcher. It runs
JavaScript plugins in isolated QuickJS-NG processes and connects them to Aura through Bridge ABI 1.

The first supported release line targets Aura `>=27.1-0-next` on Windows, Linux, and macOS for x64
and arm64. The repository is licensed under GPL-3.0-or-later.

## Source support and published packages

Current source supports canonical launch Hooks (`hook.before-game-launch`) and Runtime Patch
callbacks (`aura.patch.v1`). The Host advertises `bridge/hooks/patches/native`; its Java Provider
uses Aura's shared process supervisor and Hook codec, preserves the caller's timeout, and keeps
capability tokens inside the JVM. Provider ABI 1, Bridge ABI 1, and process protocol v1 are unchanged.

These are **source changes, not an update to the already published `0.1.0-beta.1` downloads**.
Existing Release assets and Store entries remain unchanged. Use a build from this source revision
with the pinned Aura build below when trying the new [Hook/Patch example](examples/launch-hook/README.md).

当前源码已补齐 Hook/Patch 回调支持；已发布的 beta 下载包不会因源码更新而自动获得这些能力。
本轮只交付源码和 CI 构建，不新增 Release、版本标签或 Store 条目。

## Build and verify the current source

New builds use Aura commit `636b06aad03c5d21946369c836280c891c13054d`, successful Java CI run
`33931508945`, and its unmodified `Aura-Launcher-27.1.dev-636b06a-next.jar`:

```text
SHA-256: 674f717f5f97a5b7e8f7f20e4d60aa2e25451d71a96ab475f4595d0482f99d4b
Size: 16265195 bytes
```

Use Rust from `rust-toolchain.toml`, Gradle 9.6.1, JDK 17+, Node.js/npm, PowerShell, and a native C
compiler with libclang. On Windows, initialize MSVC and point `LIBCLANG_PATH` at your existing
libclang directory for the current shell; no machine-wide environment change is required.
After placing the verified JAR under `.ci/aura/`, these PowerShell commands exercise a native build:

```powershell
$env:AURA_JAR = (Resolve-Path .ci/aura/Aura-Launcher-27.1.dev-636b06a-next.jar).Path
cargo test --workspace --locked
cargo build --release --locked -p aura-quickjs-host
$hostName = if ([Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([Runtime.InteropServices.OSPlatform]::Windows)) {
    'aura-quickjs-host.exe'
} else { 'aura-quickjs-host' }
$env:AURA_QUICKJS_PROCESS_HOST = (Resolve-Path (Join-Path target/release $hostName)).Path
gradle -p host-plugin test jar --rerun-tasks --no-daemon
```

Java tests require a real native Host and fail if it is absent; they do not silently skip integration.
The six-platform CI builds the native executable before running Java tests and checks the produced
NPL against missing capabilities, invalid manifests, and corrupt archives. CI artifacts are testing
inputs, not a new published beta. A regression can be rolled back with a normal revert commit and
the same checks; no force push or replacement of published assets is needed.
