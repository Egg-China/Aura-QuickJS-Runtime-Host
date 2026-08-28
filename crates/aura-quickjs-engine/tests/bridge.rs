use aura_bridge_value::{Error, ErrorCode, HandleValue, Value};
use aura_quickjs_engine::QuickJsPlugin;
use aura_runtime_protocol::{BridgeError, BridgeTransport};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn nested_bridge_invoke_resolves_with_the_callback_value() {
    let package = package(
        "import { bridge } from 'aura:runtime';\n\
         export async function load(context) {\n\
           if (context.bridge !== bridge) throw new Error();\n\
           if (!Object.isFrozen(context) || !Object.isFrozen(context.bridge)) throw new Error();\n\
           if (Reflect.ownKeys(context).join(',') !== 'pluginId,bridge') throw new Error();\n\
         }\n\
         export async function enable() {}\n\
         export async function invoke(_operation, input) { return await bridge.invoke('launcher.test.echo', input); }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    );
    let transport = Arc::new(RecordingBridge::default());
    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        11,
        13,
        transport.clone(),
    )
    .expect("load plugin");
    plugin.enable().expect("enable plugin");
    let input = Value::Map(vec![(
        "message".to_owned(),
        Value::String("Aura".to_owned()),
    )]);
    assert_eq!(
        plugin.invoke("hook", &input, 17).expect("invoke plugin"),
        input
    );
    assert_eq!(
        transport
            .operations
            .lock()
            .expect("lock operations")
            .as_slice(),
        ["launcher.test.echo"]
    );
}

#[test]
fn bridge_denial_rejects_with_a_stable_aura_error() {
    let package = package(
        "import { bridge, AuraError } from 'aura:runtime';\n\
         export async function load() {}\n\
         export async function enable() {}\n\
         export async function invoke(_operation, input) {\n\
           try { await bridge.invoke('launcher.test.denied', input); }\n\
           catch (error) { if (!(error instanceof AuraError)) throw error; return error; }\n\
         }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    );
    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        19,
        23,
        Arc::new(DeniedBridge),
    )
    .expect("load plugin");
    plugin.enable().expect("enable plugin");
    assert_eq!(
        plugin
            .invoke("hook", &Value::Null, 29)
            .expect("return stable error"),
        Value::Error(Error::new(ErrorCode::PermissionDenied))
    );
}

#[test]
fn retain_and_release_preserve_handle_identity() {
    let package = package(
        "import { bridge } from 'aura:runtime';\n\
         export async function load() {}\n\
         export async function enable() {}\n\
         export async function invoke(_operation, handle) {\n\
           await bridge.retain(handle); await bridge.release(handle); return handle;\n\
         }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    );
    let transport = Arc::new(RecordingBridge::default());
    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        31,
        37,
        transport.clone(),
    )
    .expect("load plugin");
    plugin.enable().expect("enable plugin");
    let handle = Value::Handle(HandleValue::new(41, 43, "launcher.profile").expect("valid handle"));
    assert_eq!(
        plugin.invoke("hook", &handle, 47).expect("invoke plugin"),
        handle
    );
    assert_eq!(
        *transport.handles.lock().expect("lock handles"),
        [("retain", 37, 41, 43), ("release", 37, 41, 43)]
    );
}

fn package(source: &str) -> tempfile::TempDir {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(package.path().join("main.mjs"), source).expect("write module");
    package
}

#[derive(Default)]
struct RecordingBridge {
    operations: Mutex<Vec<String>>,
    handles: Mutex<Vec<(&'static str, u64, u64, u64)>>,
}

impl BridgeTransport for RecordingBridge {
    fn invoke(
        &self,
        _plugin_id: u64,
        _session: u64,
        operation: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        self.operations
            .lock()
            .expect("lock operations")
            .push(operation.to_owned());
        Ok(input.to_vec())
    }

    fn retain_handle(
        &self,
        session: u64,
        object_id: u64,
        generation: u64,
    ) -> Result<(), BridgeError> {
        self.handles
            .lock()
            .expect("lock handles")
            .push(("retain", session, object_id, generation));
        Ok(())
    }

    fn release_handle(
        &self,
        session: u64,
        object_id: u64,
        generation: u64,
    ) -> Result<(), BridgeError> {
        self.handles
            .lock()
            .expect("lock handles")
            .push(("release", session, object_id, generation));
        Ok(())
    }
}

struct DeniedBridge;

impl BridgeTransport for DeniedBridge {
    fn invoke(
        &self,
        _plugin_id: u64,
        _session: u64,
        _operation: &str,
        _input: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError::Callback("permission-denied".to_owned()))
    }

    fn retain_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        Err(BridgeError::Callback("permission-denied".to_owned()))
    }

    fn release_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        Err(BridgeError::Callback("permission-denied".to_owned()))
    }
}
