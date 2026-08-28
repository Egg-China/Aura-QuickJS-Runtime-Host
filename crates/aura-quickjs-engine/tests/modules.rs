use aura_quickjs_engine::QuickJsPlugin;
use aura_runtime_protocol::{BridgeError, BridgeTransport};
use std::path::Path;
use std::sync::Arc;

#[test]
fn loads_relative_es_modules_beneath_the_package() {
    let package = package(
        "import { answer } from './lib/answer.mjs';\n\
         export async function load() { if (answer !== 42) throw new Error('bad import'); }\n\
         export async function enable() {}\n\
         export async function invoke(_operation, input) { return input; }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    );
    std::fs::create_dir(package.path().join("lib")).expect("create module directory");
    std::fs::write(
        package.path().join("lib/answer.mjs"),
        "export const answer = 42;",
    )
    .expect("write dependency");

    let mut plugin = load(package.path()).expect("load module graph");
    plugin.enable().expect("enable plugin");
    plugin.disable().expect("disable plugin");
    plugin.unload().expect("unload plugin");
}

#[test]
fn rejects_bare_and_outside_imports() {
    let bare = package("import 'node:fs';\n".to_owned() + &exports());
    assert_eq!(
        load(bare.path()).expect_err("reject bare import").code(),
        "invalid-module"
    );

    let parent = tempfile::tempdir().expect("create parent fixture");
    let root = parent.path().join("payload");
    std::fs::create_dir(&root).expect("create payload root");
    std::fs::write(parent.path().join("outside.mjs"), exports()).expect("write outside module");
    std::fs::write(
        root.join("main.mjs"),
        "import '../outside.mjs';\n".to_owned() + &exports(),
    )
    .expect("write root module");
    assert_eq!(
        load(&root).expect_err("reject parent import").code(),
        "path-escape"
    );
}

#[test]
fn rejects_a_root_module_with_a_missing_lifecycle_export() {
    let package = package(
        "export async function load() {}\n\
         export async function enable() {}\n\
         export async function invoke(_operation, input) { return input; }\n\
         export async function disable() {}",
    );
    assert_eq!(
        load(package.path())
            .expect_err("missing unload must fail")
            .code(),
        "invalid-export"
    );
}

#[test]
fn resolves_dynamic_imports_with_the_same_package_policy() {
    let package = package(
        "export async function load() {}\n\
         export async function enable() {\n\
           const dependency = await import('./dynamic.mjs');\n\
           if (dependency.answer !== 42) throw new Error('bad dynamic import');\n\
         }\n\
         export async function invoke(_operation, input) { return input; }\n\
         export async function disable() {}\n\
         export async function unload() {}",
    );
    std::fs::write(
        package.path().join("dynamic.mjs"),
        "export const answer = 42;",
    )
    .expect("write dynamic dependency");

    let mut plugin = load(package.path()).expect("load plugin");
    plugin.enable().expect("resolve dynamic import");
    plugin.disable().expect("disable plugin");
    plugin.unload().expect("unload plugin");
}

fn load(root: &Path) -> Result<QuickJsPlugin, aura_quickjs_engine::EngineError> {
    QuickJsPlugin::load(root, Path::new("main.mjs"), 11, 13, Arc::new(NoBridge))
}

fn package(source: impl AsRef<str>) -> tempfile::TempDir {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(package.path().join("main.mjs"), source.as_ref()).expect("write root module");
    package
}

fn exports() -> String {
    "export async function load() {}\n\
     export async function enable() {}\n\
     export async function invoke(_operation, input) { return input; }\n\
     export async function disable() {}\n\
     export async function unload() {}"
        .to_owned()
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
