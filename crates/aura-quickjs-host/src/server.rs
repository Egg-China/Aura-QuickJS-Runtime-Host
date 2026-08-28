use crate::{HostError, HostResult, PayloadDescriptor};
use aura_runtime_protocol::{
    BridgeError, BridgeTransport, Message, MessageBody, ProtocolError, read_frame, write_frame,
};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Language engine lifecycle consumed by the process protocol server.
pub trait GuestEngine {
    /// Loads one validated payload without enabling it.
    fn load(
        &mut self,
        package_root: &Path,
        descriptor: &PayloadDescriptor,
        plugin_id: u64,
        session: u64,
        bridge: Arc<dyn BridgeTransport>,
    ) -> HostResult<()>;

    /// Enables the loaded payload.
    fn enable(&mut self) -> HostResult<()>;

    /// Invokes one operation while the payload is enabled.
    fn invoke(&mut self, operation: &str, input: &[u8], callback_id: u64) -> HostResult<Vec<u8>>;

    /// Disables the enabled payload.
    fn disable(&mut self) -> HostResult<()>;

    /// Unloads the payload and releases language resources.
    fn unload(&mut self) -> HostResult<()>;
}

/// Serves one isolated payload over process protocol v1.
pub struct ProcessServer<R, W, E> {
    reader: Arc<Mutex<R>>,
    writer: Arc<Mutex<W>>,
    poison: Arc<Mutex<Option<&'static str>>>,
    engine: E,
    state: LifecycleState,
}

impl<R, W, E> ProcessServer<R, W, E>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    E: GuestEngine,
{
    /// Creates a server with exclusive input, output, and engine ownership.
    #[must_use]
    pub fn new(reader: R, writer: W, engine: E) -> Self {
        Self {
            reader: Arc::new(Mutex::new(reader)),
            writer: Arc::new(Mutex::new(writer)),
            poison: Arc::new(Mutex::new(None)),
            engine,
            state: LifecycleState::AwaitHello,
        }
    }

    /// Processes frames until clean EOF or successful shutdown.
    pub fn serve(mut self) -> Result<(), ProtocolError> {
        loop {
            let message = {
                let mut reader = self
                    .reader
                    .lock()
                    .map_err(|_| invalid_protocol("process reader lock is poisoned"))?;
                read_frame(&mut *reader)?
            };
            let Some(message) = message else {
                self.close_on_eof();
                return Ok(());
            };
            if message.request_id().is_multiple_of(2) {
                return Err(invalid_protocol(
                    "callback response arrived outside a callback",
                ));
            }
            let request_id = message.request_id();
            let handled = self.handle(message.body().clone());
            if let Some(reason) = *self
                .poison
                .lock()
                .map_err(|_| invalid_protocol("process poison lock is poisoned"))?
            {
                return Err(invalid_protocol(reason));
            }
            let (response, close) = handled?;
            {
                let mut writer = self
                    .writer
                    .lock()
                    .map_err(|_| invalid_protocol("process writer lock is poisoned"))?;
                write_frame(
                    &mut *writer,
                    &Message::new(request_id, response)
                        .map_err(|_| invalid_protocol("invalid process response"))?,
                )?;
                writer.flush()?;
            }
            if close {
                return Ok(());
            }
        }
    }

    fn handle(&mut self, body: MessageBody) -> Result<(MessageBody, bool), ProtocolError> {
        let result = match body {
            MessageBody::Hello => self.hello().map(|()| (MessageBody::Ok, false)),
            MessageBody::Load {
                package_root,
                entrypoint,
                plugin_id,
                session,
            } => self
                .load(&package_root, &entrypoint, plugin_id, session)
                .map(|()| (MessageBody::Ok, false)),
            MessageBody::Enable => self.enable().map(|()| (MessageBody::Ok, false)),
            MessageBody::Invoke {
                operation,
                input,
                callback_id,
            } => self
                .invoke(&operation, &input, callback_id)
                .map(|output| (MessageBody::Result { output }, false)),
            MessageBody::Disable => self.disable().map(|()| (MessageBody::Ok, false)),
            MessageBody::Shutdown => self.shutdown().map(|()| (MessageBody::Ok, true)),
            MessageBody::Ok
            | MessageBody::Result { .. }
            | MessageBody::Error { .. }
            | MessageBody::BridgeInvoke { .. }
            | MessageBody::RetainHandle { .. }
            | MessageBody::ReleaseHandle { .. }
            | MessageBody::CallbackResult { .. }
            | MessageBody::CallbackError { .. } => {
                return Err(invalid_protocol(
                    "child-only message arrived as a parent command",
                ));
            }
        };
        match result {
            Ok(response) => Ok(response),
            Err(error) if error.is_fatal() => Err(invalid_protocol(error.code())),
            Err(error) => Ok((error_body(error), false)),
        }
    }

    fn hello(&mut self) -> HostResult<()> {
        self.require(LifecycleState::AwaitHello)?;
        self.state = LifecycleState::AwaitLoad;
        Ok(())
    }

    fn load(
        &mut self,
        package_root: &str,
        entrypoint: &str,
        plugin_id: u64,
        session: u64,
    ) -> HostResult<()> {
        self.require(LifecycleState::AwaitLoad)?;
        let descriptor = PayloadDescriptor::read(Path::new(package_root), entrypoint)?;
        let bridge = Arc::new(ProcessBridge {
            reader: Arc::clone(&self.reader),
            writer: Arc::clone(&self.writer),
            plugin_id,
            session,
            next_callback_id: AtomicU64::new(2),
            poison: Arc::clone(&self.poison),
        });
        self.engine.load(
            Path::new(package_root),
            &descriptor,
            plugin_id,
            session,
            bridge,
        )?;
        self.state = LifecycleState::Loaded;
        Ok(())
    }

    fn enable(&mut self) -> HostResult<()> {
        self.require(LifecycleState::Loaded)?;
        self.engine.enable()?;
        self.state = LifecycleState::Enabled;
        Ok(())
    }

    fn invoke(&mut self, operation: &str, input: &[u8], callback_id: u64) -> HostResult<Vec<u8>> {
        self.require(LifecycleState::Enabled)?;
        self.engine.invoke(operation, input, callback_id)
    }

    fn disable(&mut self) -> HostResult<()> {
        self.require(LifecycleState::Enabled)?;
        self.engine.disable()?;
        self.state = LifecycleState::Disabled;
        Ok(())
    }

    fn shutdown(&mut self) -> HostResult<()> {
        if !matches!(
            self.state,
            LifecycleState::Loaded | LifecycleState::Disabled
        ) {
            return Err(invalid_state());
        }
        self.engine.unload()?;
        self.state = LifecycleState::Closed;
        Ok(())
    }

    fn require(&self, expected: LifecycleState) -> HostResult<()> {
        if self.state == expected {
            Ok(())
        } else {
            Err(invalid_state())
        }
    }

    fn close_on_eof(&mut self) {
        if self.state == LifecycleState::Enabled {
            let _ = self.engine.disable();
        }
        if matches!(
            self.state,
            LifecycleState::Loaded | LifecycleState::Enabled | LifecycleState::Disabled
        ) {
            let _ = self.engine.unload();
        }
        self.state = LifecycleState::Closed;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleState {
    AwaitHello,
    AwaitLoad,
    Loaded,
    Enabled,
    Disabled,
    Closed,
}

struct ProcessBridge<R, W> {
    reader: Arc<Mutex<R>>,
    writer: Arc<Mutex<W>>,
    plugin_id: u64,
    session: u64,
    next_callback_id: AtomicU64,
    poison: Arc<Mutex<Option<&'static str>>>,
}

impl<R, W> ProcessBridge<R, W>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    fn call(&self, body: MessageBody) -> Result<MessageBody, BridgeError> {
        let request_id = self.next_callback_id.fetch_add(2, Ordering::Relaxed);
        if request_id == 0 || request_id > i64::MAX as u64 {
            return Err(self.protocol_failure(invalid_protocol("callback request ID overflow")));
        }
        let message =
            Message::new(request_id, body).map_err(|error| self.protocol_failure(error))?;
        {
            let mut writer = self.writer.lock().map_err(|_| {
                self.protocol_failure(invalid_protocol("process writer lock is poisoned"))
            })?;
            write_frame(&mut *writer, &message).map_err(|error| self.protocol_failure(error))?;
            writer
                .flush()
                .map_err(ProtocolError::Io)
                .map_err(|error| self.protocol_failure(error))?;
        }
        let response = {
            let mut reader = self.reader.lock().map_err(|_| {
                self.protocol_failure(invalid_protocol("process reader lock is poisoned"))
            })?;
            read_frame(&mut *reader).map_err(|error| self.protocol_failure(error))?
        }
        .ok_or_else(|| self.protocol_failure(invalid_protocol("callback response is missing")))?;
        if response.request_id() != request_id {
            return Err(
                self.protocol_failure(invalid_protocol("callback response request ID mismatch"))
            );
        }
        Ok(response.body().clone())
    }

    fn protocol_failure(&self, error: ProtocolError) -> BridgeError {
        let reason = match &error {
            ProtocolError::InvalidData(reason) => *reason,
            ProtocolError::Io(_) => "Bridge callback I/O failure",
        };
        if let Ok(mut poison) = self.poison.lock()
            && poison.is_none()
        {
            *poison = Some(reason);
        }
        BridgeError::Protocol(error)
    }

    fn handle_call(&self, session: u64, body: MessageBody) -> Result<(), BridgeError> {
        if session != self.session {
            return Err(self.protocol_failure(invalid_protocol("callback session mismatch")));
        }
        match self.call(body)? {
            MessageBody::CallbackResult { output } if output.is_empty() => Ok(()),
            MessageBody::CallbackError { code } => Err(BridgeError::Callback(code)),
            _ => Err(self.protocol_failure(invalid_protocol("invalid handle callback response"))),
        }
    }
}

impl<R, W> BridgeTransport for ProcessBridge<R, W>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    fn invoke(
        &self,
        plugin_id: u64,
        session: u64,
        operation: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        if plugin_id != self.plugin_id || session != self.session {
            return Err(self.protocol_failure(invalid_protocol("callback owner mismatch")));
        }
        match self.call(MessageBody::BridgeInvoke {
            operation: operation.to_owned(),
            input: input.to_vec(),
        })? {
            MessageBody::CallbackResult { output } => Ok(output),
            MessageBody::CallbackError { code } => Err(BridgeError::Callback(code)),
            _ => Err(self.protocol_failure(invalid_protocol("invalid Bridge callback response"))),
        }
    }

    fn retain_handle(
        &self,
        session: u64,
        object_id: u64,
        generation: u64,
    ) -> Result<(), BridgeError> {
        self.handle_call(
            session,
            MessageBody::RetainHandle {
                object_id,
                generation,
            },
        )
    }

    fn release_handle(
        &self,
        session: u64,
        object_id: u64,
        generation: u64,
    ) -> Result<(), BridgeError> {
        self.handle_call(
            session,
            MessageBody::ReleaseHandle {
                object_id,
                generation,
            },
        )
    }
}

fn error_body(error: HostError) -> MessageBody {
    MessageBody::Error {
        code: error.code().to_owned(),
        message: error.to_string(),
    }
}

fn invalid_state() -> HostError {
    HostError::new(
        "invalid-state",
        "lifecycle command is not valid in the current state",
    )
}

const fn invalid_protocol(message: &'static str) -> ProtocolError {
    ProtocolError::InvalidData(message)
}
