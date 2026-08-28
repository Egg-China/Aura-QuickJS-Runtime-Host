use aura_quickjs_host::{GuestEngine, HostResult, PayloadDescriptor, ProcessServer};
use aura_runtime_protocol::{BridgeTransport, Message, MessageBody, read_frame, write_frame};
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn invoke_before_enable_returns_invalid_state() {
    let package = package();
    let input = framed([
        message(1, MessageBody::Hello),
        load(3, package.path()),
        message(
            5,
            MessageBody::Invoke {
                operation: "echo".into(),
                input: vec![1, 2, 3],
                callback_id: 0,
            },
        ),
    ]);
    let output = SharedOutput::default();
    ProcessServer::new(
        Cursor::new(input),
        output.clone(),
        RecordingEngine::default(),
    )
    .serve()
    .expect("serve commands");

    let responses = output.messages();
    assert_eq!(responses.len(), 3);
    assert!(
        matches!(responses[2].body(), MessageBody::Error { code, .. } if code == "invalid-state")
    );
}

#[test]
fn complete_lifecycle_returns_guest_result_and_unloads() {
    let package = package();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let input = framed([
        message(1, MessageBody::Hello),
        load(3, package.path()),
        message(5, MessageBody::Enable),
        message(
            7,
            MessageBody::Invoke {
                operation: "echo".into(),
                input: vec![4, 5],
                callback_id: 9,
            },
        ),
        message(9, MessageBody::Disable),
        message(11, MessageBody::Shutdown),
    ]);
    let output = SharedOutput::default();
    ProcessServer::new(
        Cursor::new(input),
        output.clone(),
        RecordingEngine {
            calls: Arc::clone(&calls),
        },
    )
    .serve()
    .expect("serve lifecycle");

    assert_eq!(
        *calls.lock().expect("lock calls"),
        ["load", "enable", "invoke", "disable", "unload"]
    );
    let responses = output.messages();
    assert!(matches!(&responses[3].body(), MessageBody::Result { output } if output == &[4, 5]));
}

#[derive(Clone, Default)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl SharedOutput {
    fn messages(&self) -> Vec<Message> {
        let bytes = self.0.lock().expect("lock output").clone();
        let mut reader = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = read_frame(&mut reader).expect("decode response") {
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

#[derive(Default)]
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
        self.calls.lock().expect("lock calls").push("invoke");
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

fn package() -> tempfile::TempDir {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("aura-javascript.json"),
        r#"{"schemaVersion":1,"module":"main.mjs"}"#,
    )
    .expect("write descriptor");
    std::fs::write(
        package.path().join("main.mjs"),
        "export async function load() {}",
    )
    .expect("write module");
    package
}

fn load(request_id: u64, package_root: &Path) -> Message {
    message(
        request_id,
        MessageBody::Load {
            package_root: package_root.to_string_lossy().into_owned(),
            entrypoint: "aura-javascript.json".into(),
            plugin_id: 11,
            session: 13,
        },
    )
}

fn message(request_id: u64, body: MessageBody) -> Message {
    Message::new(request_id, body).expect("valid message")
}

fn framed<const N: usize>(messages: [Message; N]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for message in messages {
        write_frame(&mut bytes, &message).expect("encode frame");
    }
    bytes
}
