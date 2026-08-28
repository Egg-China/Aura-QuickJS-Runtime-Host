use aura_quickjs_host::{HostError, PackagePathPolicy, PayloadDescriptor};
use std::fs;
use std::io;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn reads_the_exact_v1_descriptor() {
    let package = fixture(r#"{"schemaVersion":1,"module":"main.mjs"}"#);
    let descriptor = PayloadDescriptor::read(package.path(), "aura-javascript.json")
        .expect("read exact descriptor");
    assert_eq!(descriptor.module(), Path::new("main.mjs"));
}

#[test]
fn rejects_unknown_fields_and_parent_segments() {
    let unknown = fixture(r#"{"schemaVersion":1,"module":"main.mjs","extra":true}"#);
    assert_code(
        PayloadDescriptor::read(unknown.path(), "aura-javascript.json"),
        "invalid-descriptor",
    );

    let parent = fixture(r#"{"schemaVersion":1,"module":"../main.mjs"}"#);
    assert_code(
        PayloadDescriptor::read(parent.path(), "aura-javascript.json"),
        "path-escape",
    );
}

#[test]
fn resolves_only_relative_mjs_modules_beneath_the_package() {
    let package = fixture(r#"{"schemaVersion":1,"module":"modules/main.mjs"}"#);
    fs::write(
        package.path().join("modules/dependency.mjs"),
        "export default 1",
    )
    .expect("write dependency");
    let policy = PackagePathPolicy::new(package.path()).expect("create policy");

    let resolved = policy
        .resolve_module("./dependency.mjs", Some(Path::new("modules/main.mjs")))
        .expect("resolve dependency");
    assert_eq!(resolved, package.path().join("modules/dependency.mjs"));
    assert_code(policy.resolve_module("node:fs", None), "invalid-module");
    assert_code(policy.resolve_module("../outside.mjs", None), "path-escape");
}

#[test]
fn rejects_a_descriptor_symlink_that_escapes_the_package() {
    let package = tempfile::tempdir().expect("create package");
    let outside = tempfile::tempdir().expect("create outside directory");
    let outside_descriptor = outside.path().join("outside.json");
    fs::write(
        &outside_descriptor,
        r#"{"schemaVersion":1,"module":"main.mjs"}"#,
    )
    .expect("write outside descriptor");
    fs::write(
        package.path().join("main.mjs"),
        "export async function load() {}",
    )
    .expect("write module");
    let link = package.path().join("aura-javascript.json");
    if let Err(error) = create_file_symlink(&outside_descriptor, &link) {
        if error.kind() == io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("create descriptor symlink: {error}");
    }

    assert_code(
        PayloadDescriptor::read(package.path(), "aura-javascript.json"),
        "path-escape",
    );
}

fn fixture(descriptor: &str) -> TempDir {
    let directory = tempfile::tempdir().expect("create fixture");
    fs::write(directory.path().join("aura-javascript.json"), descriptor).expect("write descriptor");
    fs::write(
        directory.path().join("main.mjs"),
        "export async function load() {}",
    )
    .expect("write module");
    fs::create_dir_all(directory.path().join("modules")).expect("create modules");
    fs::write(
        directory.path().join("modules/main.mjs"),
        "export { default } from './dependency.mjs'",
    )
    .expect("write nested module");
    directory
}

fn assert_code<T>(result: Result<T, HostError>, expected: &str) {
    let error = result.err().expect("operation must fail");
    assert_eq!(error.code(), expected);
}

#[cfg(unix)]
fn create_file_symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn create_file_symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}
