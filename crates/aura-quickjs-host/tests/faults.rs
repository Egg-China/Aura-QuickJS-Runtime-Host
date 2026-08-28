use aura_bridge_value::Value;
use aura_quickjs_host::{
    GuestEngine, HostError, HostResult, PayloadDescriptor, ProcessServer, QuickJsGuestEngine,
};
use aura_runtime_protocol::{BridgeTransport, Message, MessageBody, read_frame, write_frame};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[test]
fn wrong_bridge_callback_id_poisoning_is_fatal() {
    let package = fixture("bridge-callback");
    let wire = Value::Null.to_wire().expect("encode input");
    let input = framed(&[
        message(1, MessageBody::Hello),
        load(3, &package),
        message(5, MessageBody::Enable),
        message(
            7,
            MessageBody::Invoke {
                operation: "hook".to_owned(),
                input: wire.clone(),
                callback_id: 17,
            },
        ),
        message(4, MessageBody::CallbackResult { output: wire }),
    ]);
    let output = SharedOutput::default();

    let error = ProcessServer::new(
        Cursor::new(input),
        output.clone(),
        QuickJsGuestEngine::default(),
    )
    .serve()
    .expect_err("wrong callback ID must poison the process");

    assert!(
        error
            .to_string()
            .contains("callback response request ID mismatch")
    );
    assert_eq!(output.messages().len(), 4);
    assert!(matches!(
        output.messages()[3].body(),
        MessageBody::BridgeInvoke { .. }
    ));
}

#[test]
fn fatal_engine_failure_emits_no_recoverable_command_error() {
    for code in ["deadline-exceeded", "resource-limit", "runtime-failure"] {
        let package = package();
        let input = framed(&[
            message(1, MessageBody::Hello),
            load(3, package.path()),
            message(5, MessageBody::Enable),
        ]);
        let output = SharedOutput::default();
        let error = ProcessServer::new(Cursor::new(input), output.clone(), FailingEngine { code })
            .serve()
            .expect_err("fatal engine error must terminate the process protocol");

        assert!(error.to_string().contains(code));
        assert_eq!(
            output.messages().len(),
            2,
            "fatal {code} was written as a response"
        );
    }
}

#[test]
fn clean_eof_disables_and_unloads_an_enabled_payload() {
    let package = package();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let input = framed(&[
        message(1, MessageBody::Hello),
        load(3, package.path()),
        message(5, MessageBody::Enable),
    ]);

    ProcessServer::new(
        Cursor::new(input),
        SharedOutput::default(),
        RecordingEngine {
            calls: Arc::clone(&calls),
        },
    )
    .serve()
    .expect("clean EOF must perform best-effort cleanup");

    assert_eq!(
        *calls.lock().expect("lock calls"),
        ["load", "enable", "disable", "unload"]
    );
}

#[test]
fn invalid_utf8_source_is_rejected_before_enable() {
    let package = package();
    std::fs::write(package.path().join("main.mjs"), [0xff, 0xfe, 0xfd])
        .expect("write invalid UTF-8 module");
    let output = SharedOutput::default();
    ProcessServer::new(
        Cursor::new(framed(&[
            message(1, MessageBody::Hello),
            load(3, package.path()),
        ])),
        output.clone(),
        QuickJsGuestEngine::default(),
    )
    .serve()
    .expect("invalid source is a bounded load failure");

    assert!(matches!(
        output.messages()[1].body(),
        MessageBody::Error { code, .. } if code == "invalid-module"
    ));
}

#[test]
fn unavailable_console_cannot_write_unframed_stdout() {
    let package = fixture("stdout-log");
    let output = SharedOutput::default();
    ProcessServer::new(
        Cursor::new(framed(&[
            message(1, MessageBody::Hello),
            load(3, &package),
            message(5, MessageBody::Enable),
        ])),
        output.clone(),
        QuickJsGuestEngine::default(),
    )
    .serve()
    .expect("missing console is a bounded guest exception");

    let messages = output.messages();
    assert_eq!(messages.len(), 3);
    assert!(matches!(
        messages[2].body(),
        MessageBody::Error { code, .. } if code == "guest-exception"
    ));
}

struct FailingEngine {
    code: &'static str,
}

impl GuestEngine for FailingEngine {
    fn load(
        &mut self,
        _package_root: &Path,
        _descriptor: &PayloadDescriptor,
        _plugin_id: u64,
        _session: u64,
        _bridge: Arc<dyn BridgeTransport>,
    ) -> HostResult<()> {
        Ok(())
    }

    fn enable(&mut self) -> HostResult<()> {
        Err(HostError::new(self.code, self.code))
    }

    fn invoke(
        &mut self,
        _operation: &str,
        _input: &[u8],
        _callback_id: u64,
    ) -> HostResult<Vec<u8>> {
        unreachable!("failing engine never reaches invoke")
    }

    fn disable(&mut self) -> HostResult<()> {
        Ok(())
    }

    fn unload(&mut self) -> HostResult<()> {
        Ok(())
    }
}

struct RecordingEngine {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl GuestEngine for RecordingEngine {
    fn load(
        &mut self,
        _package_root: &Path,
        _descriptor: &PayloadDescriptor,
        _plugin_id: u64,
        _session: u64,
        _bridge: Arc<dyn BridgeTransport>,
    ) -> HostResult<()> {
        self.calls.lock().expect("lock calls").push("load");
        Ok(())
    }

    fn enable(&mut self) -> HostResult<()> {
        self.calls.lock().expect("lock calls").push("enable");
        Ok(())
    }

    fn invoke(&mut self, _operation: &str, input: &[u8], _callback_id: u64) -> HostResult<Vec<u8>> {
        Ok(input.to_vec())
    }

    fn disable(&mut self) -> HostResult<()> {
        self.calls.lock().expect("lock calls").push("disable");
        Ok(())
    }

    fn unload(&mut self) -> HostResult<()> {
        self.calls.lock().expect("lock calls").push("unload");
        Ok(())
    }
}

#[derive(Clone, Default)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl SharedOutput {
    fn messages(&self) -> Vec<Message> {
        let bytes = self.0.lock().expect("lock output").clone();
        let mut reader = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = read_frame(&mut reader).expect("decode framed stdout") {
            messages.push(message);
        }
        messages
    }
}

impl Write for SharedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock output").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
        .canonicalize()
        .expect("locate fault fixture")
}

fn package() -> tempfile::TempDir {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("aura-javascript.json"),
        r#"{"schemaVersion":1,"module":"main.mjs"}"#,
    )
    .expect("write descriptor");
    std::fs::write(
        package.path().join("main.mjs"),
        "export function load() {}\n\
         export function enable() {}\n\
         export function invoke(_operation, input) { return input; }\n\
         export function disable() {}\n\
         export function unload() {}",
    )
    .expect("write module");
    package
}

fn load(request_id: u64, package_root: &Path) -> Message {
    message(
        request_id,
        MessageBody::Load {
            package_root: package_root.to_string_lossy().into_owned(),
            entrypoint: "aura-javascript.json".to_owned(),
            plugin_id: 11,
            session: 13,
        },
    )
}

fn message(request_id: u64, body: MessageBody) -> Message {
    Message::new(request_id, body).expect("valid message")
}

fn framed(messages: &[Message]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for message in messages {
        write_frame(&mut bytes, message).expect("encode frame");
    }
    bytes
}
