use aura_quickjs_engine::QuickJsPlugin;
use aura_runtime_protocol::{BridgeError, BridgeTransport};
use std::path::Path;
use std::sync::Arc;

#[test]
fn executes_async_lifecycle_functions_in_order() {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("main.mjs"),
        "let state = 0;\n\
         export async function load() { if (state !== 0) throw new Error(); state = 1; }\n\
         export async function enable() { if (state !== 1) throw new Error(); await Promise.resolve(); state = 2; }\n\
         export async function invoke(_operation, input) { if (state !== 2) throw new Error(); return input; }\n\
         export async function disable() { if (state !== 2) throw new Error(); state = 3; }\n\
         export async function unload() { if (state !== 3) throw new Error(); state = 4; }",
    )
    .expect("write module");

    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        17,
        19,
        Arc::new(NoBridge),
    )
    .expect("load plugin");
    plugin.enable().expect("enable plugin");
    plugin.disable().expect("disable plugin");
    plugin.unload().expect("unload plugin");
}

#[test]
fn maps_a_rejected_lifecycle_promise_to_guest_exception() {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("main.mjs"),
        "export async function load() {}\n\
         export async function enable() { throw new Error('payload-private-message'); }\n\
         export async function invoke(_operation, input) { return input; }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    )
    .expect("write module");

    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        23,
        29,
        Arc::new(NoBridge),
    )
    .expect("load plugin");
    let error = plugin.enable().expect_err("enable must reject");
    assert_eq!(error.code(), "guest-exception");
    assert_eq!(error.to_string(), "guest-exception");
}

#[test]
fn accepts_synchronous_lifecycle_without_installing_host_apis() {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("main.mjs"),
        "export function load() {\n\
           if (typeof process !== 'undefined') throw new Error('process exposed');\n\
           if (typeof require !== 'undefined') throw new Error('require exposed');\n\
           if (typeof fetch !== 'undefined') throw new Error('fetch exposed');\n\
         }\n\
         export function enable() {}\n\
         export function invoke(_operation, input) { return input; }\n\
         export function disable() {}\n\
         export function unload() {}",
    )
    .expect("write module");

    let mut plugin = QuickJsPlugin::load(
        package.path(),
        Path::new("main.mjs"),
        31,
        37,
        Arc::new(NoBridge),
    )
    .expect("load synchronous plugin");
    plugin.enable().expect("enable plugin");
    plugin.disable().expect("disable plugin");
    plugin.unload().expect("unload plugin");
}

#[derive(Debug)]
struct NoBridge;

impl BridgeTransport for NoBridge {
    fn invoke(
        &self,
        _plugin_id: u64,
        _session: u64,
        _operation: &str,
        _input: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        panic!("Bridge must not be called")
    }

    fn retain_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        panic!("Bridge must not be called")
    }

    fn release_handle(
        &self,
        _session: u64,
        _object_id: u64,
        _generation: u64,
    ) -> Result<(), BridgeError> {
        panic!("Bridge must not be called")
    }
}
