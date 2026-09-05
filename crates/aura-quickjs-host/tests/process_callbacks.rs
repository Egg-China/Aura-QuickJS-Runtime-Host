use aura_bridge_value::{HandleValue, Value};
use aura_runtime_protocol::{Message, MessageBody, read_frame, write_frame};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// Hand-derived Bridge Value v1 vectors: tagged map, ordered keys, signed int64.
const HOOK_UNCHANGED: &[u8] = b"\x92\x07\xdd\0\0\0\x02\x92\xdb\0\0\0\x0fcontractVersion\x92\x02\xd3\0\0\0\0\0\0\0\x01\x92\xdb\0\0\0\x06action\x92\x04\xdb\0\0\0\x09unchanged";
const PATCH_UNCHANGED: &[u8] = b"\x92\x07\xdd\0\0\0\x02\x92\xdb\0\0\0\x0dschemaVersion\x92\x02\xd3\0\0\0\0\0\0\0\x01\x92\xdb\0\0\0\x06action\x92\x04\xdb\0\0\0\x09unchanged";

#[test]
fn real_stdio_hook_and_patch_complete_lifecycle_with_exact_wire_results() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/launch-hook")
        .canonicalize()
        .expect("locate sample");
    let hook = Value::Map(vec![
        ("contractVersion".into(), Value::Integer(1)),
        ("dispatchId".into(), Value::String("quickjs-stdio".into())),
        ("point".into(), Value::String("before-game-launch".into())),
        (
            "occurredAt".into(),
            Value::String("2026-09-05T00:00:00Z".into()),
        ),
        ("data".into(), Value::Map(vec![])),
    ]);
    let patch = Value::Map(vec![
        ("schemaVersion".into(), Value::Integer(1)),
        (
            "target".into(),
            Value::String("org.jackhuang.hmcl.util.io.FileUtils".into()),
        ),
        ("method".into(), Value::String("getName".into())),
        (
            "parameters".into(),
            Value::Array(vec![Value::String("java.nio.file.Path".into())]),
        ),
        ("type".into(), Value::String("after".into())),
        ("receiver".into(), Value::Null),
        (
            "arguments".into(),
            Value::Array(vec![Value::Handle(
                HandleValue::new(1, 1, "patch-reference")
                    .expect("opaque invocation-local reference"),
            )]),
        ),
        ("result".into(), Value::String("example.jar".into())),
    ]);
    let mut input = Vec::new();
    for (id, body) in [
        (1, MessageBody::Hello),
        (
            3,
            MessageBody::Load {
                package_root: root.to_str().expect("UTF-8 sample path").into(),
                entrypoint: "aura-javascript.json".into(),
                plugin_id: 59,
                session: 61,
            },
        ),
        (5, MessageBody::Enable),
        (
            7,
            MessageBody::Invoke {
                operation: "hook.before-game-launch".into(),
                input: hook.to_wire().unwrap(),
                callback_id: 0,
            },
        ),
        (
            9,
            MessageBody::Invoke {
                operation: "aura.patch.v1".into(),
                input: patch.to_wire().unwrap(),
                callback_id: 0,
            },
        ),
        (11, MessageBody::Disable),
        (13, MessageBody::Shutdown),
    ] {
        write_frame(&mut input, &Message::new(id, body).unwrap()).unwrap();
    }
    let captured = run_host(&input, true, Duration::from_secs(10));
    assert!(!captured.timed_out, "Host exceeded test deadline");
    assert!(
        captured.status.success(),
        "Host failed: {:?}",
        captured.stderr
    );
    assert!(
        captured.stderr.is_empty(),
        "unexpected stderr: {:?}",
        captured.stderr
    );
    let mut output = captured.stdout.as_slice();
    for id in [1, 3, 5, 7, 9, 11, 13] {
        let response = read_frame(&mut output)
            .expect("strict framed stdout")
            .expect("response");
        assert_eq!(
            response.request_id(),
            id,
            "exact request/response correlation"
        );
        match id {
            7 | 9 => {
                let MessageBody::Result { output } = response.body() else {
                    panic!("expected callback result, got {response:?}");
                };
                assert_eq!(
                    output,
                    if id == 7 {
                        HOOK_UNCHANGED
                    } else {
                        PATCH_UNCHANGED
                    }
                );
            }
            _ => assert_eq!(response.body(), &MessageBody::Ok),
        }
    }
    assert!(output.is_empty(), "extra stdout after shutdown");
}

#[test]
fn incomplete_frame_hits_test_deadline_and_reaps_the_real_child() {
    let captured = run_host(&[0], false, Duration::from_millis(150));
    assert!(
        captured.timed_out,
        "partial frame must remain blocked until killed"
    );
    assert!(
        !captured.status.success(),
        "test deadline must terminate child"
    );
    assert!(captured.stdout.is_empty());
}

struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_host(input: &[u8], close_stdin: bool, timeout: Duration) -> Captured {
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_aura-quickjs-host"))
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start real QuickJS Host"),
    );
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let output_reader = thread::spawn(move || read_output(stdout));
    let error_reader = thread::spawn(move || read_output(stderr));
    let mut stdin = child.0.stdin.take().unwrap();
    stdin.write_all(input).expect("write lifecycle");
    stdin.flush().unwrap();
    let open_stdin = if close_stdin {
        drop(stdin);
        None
    } else {
        Some(stdin)
    };
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().expect("poll Host") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.0.kill().expect("terminate timed-out Host");
            break (child.0.wait().expect("reap timed-out Host"), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    drop(open_stdin);
    Captured {
        status,
        timed_out,
        stdout: output_reader.join().expect("read stdout"),
        stderr: error_reader.join().expect("read stderr"),
    }
}

fn read_output(mut stream: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("drain child stream");
    bytes
}
