# Process Protocol Provenance

The Bridge Value v1 codec and process protocol v1 were copied from
`Egg-China/Aura-Rust-Runtime-Host` commit
`8e65a577d20903ad6eb07ff2afc536c049b9e907`.

| Source | Original SHA-256 | Local change |
| --- | --- | --- |
| `crates/hmcl-plugin-sdk/src/value.rs` | `379ea742012d571ada4f69482fa260dda604cff1dfdac3ad066c4c6db85d12bd` | Moved into `aura-bridge-value`; error types detached from the Rust payload ABI. |
| `crates/hmcl-runtime-protocol/src/lib.rs` | `bb9913f17e46faae6bc19df4363b63355dc54551db35484bb728132f14b2a123` | Dependency namespace changed to `aura_bridge_value`. |

Wire tags, canonical map order, limits, request-ID parity, message kinds, frame format, and validation
rules are unchanged. Golden-vector tests guard the resulting bytes.
