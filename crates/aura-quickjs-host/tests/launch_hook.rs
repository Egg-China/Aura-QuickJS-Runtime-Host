use aura_bridge_value::Value;
use aura_quickjs_engine::QuickJsPlugin;
use aura_runtime_protocol::{BridgeError, BridgeTransport};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn launch_hook_example_returns_canonical_unchanged() {
    assert_example("hook.before-game-launch", "contractVersion");
}

#[test]
fn launch_hook_example_returns_canonical_patch_unchanged() {
    assert_example("aura.patch.v1", "schemaVersion");
}

fn assert_example(operation: &str, version_field: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/launch-hook")
        .canonicalize()
        .expect("locate launch-hook example");
    let mut plugin = QuickJsPlugin::load(&root, Path::new("main.mjs"), 59, 61, Arc::new(NoBridge))
        .expect("load launch-hook example");
    plugin.enable().expect("enable example");
    let input = Value::Map(vec![
        (
            "workingDirectory".to_owned(),
            Value::String("C:/Games/Aura".to_owned()),
        ),
        ("arguments".to_owned(), Value::Array(Vec::new())),
    ]);
    assert_eq!(
        plugin
            .invoke(operation, &input, 0)
            .expect("invoke launch Hook"),
        Value::Map(vec![
            (version_field.into(), Value::Integer(1)),
            ("action".into(), Value::String("unchanged".into())),
        ])
    );
    plugin.disable().expect("disable example");
    plugin.unload().expect("unload example");
}

struct NoBridge;

impl BridgeTransport for NoBridge {
    fn invoke(
        &self,
        _plugin_id: u64,
        _session: u64,
        _operation: &str,
        _input: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        panic!("example must not invoke Bridge")
    }

    fn retain_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        panic!("example must not retain handles")
    }

    fn release_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        panic!("example must not release handles")
    }
}
