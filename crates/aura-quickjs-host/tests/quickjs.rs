use aura_quickjs_host::{ProcessServer, QuickJsGuestEngine};
use aura_runtime_protocol::{Message, MessageBody, read_frame, write_frame};
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn drives_a_real_quickjs_payload_through_process_lifecycle() {
    let package = tempfile::tempdir().expect("create package");
    std::fs::write(
        package.path().join("aura-javascript.json"),
        r#"{"schemaVersion":1,"module":"main.mjs"}"#,
    )
    .expect("write descriptor");
    std::fs::write(
        package.path().join("main.mjs"),
        "let state = 0;\n\
         export async function load() { state = 1; }\n\
         export async function enable() { if (state !== 1) throw new Error(); state = 2; }\n\
         export async function invoke(_operation, input) { return input; }\n\
         export async function disable() { if (state !== 2) throw new Error(); state = 3; }\n\
         export async function unload() { if (state !== 3) throw new Error(); }",
    )
    .expect("write module");

    let input = framed([
        message(1, MessageBody::Hello),
        load(3, package.path()),
        message(5, MessageBody::Enable),
        message(7, MessageBody::Disable),
        message(9, MessageBody::Shutdown),
    ]);
    let output = SharedOutput::default();
    ProcessServer::new(
        Cursor::new(input),
        output.clone(),
        QuickJsGuestEngine::default(),
    )
    .serve()
    .expect("serve real payload");

    let responses = output.messages();
    assert_eq!(responses.len(), 5);
    assert!(
        responses
            .iter()
            .all(|message| matches!(message.body(), MessageBody::Ok))
    );
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

fn load(request_id: u64, package_root: &Path) -> Message {
    message(
        request_id,
        MessageBody::Load {
            package_root: package_root.to_string_lossy().into_owned(),
            entrypoint: "aura-javascript.json".into(),
            plugin_id: 41,
            session: 43,
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
