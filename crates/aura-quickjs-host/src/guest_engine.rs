use crate::{GuestEngine, HostError, HostResult, PayloadDescriptor};
use aura_bridge_value::Value;
use aura_quickjs_engine::{EngineError, QuickJsPlugin};
use aura_runtime_protocol::BridgeTransport;
use std::path::Path;
use std::sync::Arc;

/// Adapts one real QuickJS payload to the process server lifecycle.
#[derive(Default)]
pub struct QuickJsGuestEngine {
    plugin: Option<QuickJsPlugin>,
}

impl GuestEngine for QuickJsGuestEngine {
    fn load(
        &mut self,
        package_root: &Path,
        descriptor: &PayloadDescriptor,
        plugin_id: u64,
        session: u64,
        bridge: Arc<dyn BridgeTransport>,
    ) -> HostResult<()> {
        if self.plugin.is_some() {
            return Err(invalid_state());
        }
        self.plugin = Some(
            QuickJsPlugin::load(
                package_root,
                descriptor.module(),
                plugin_id,
                session,
                bridge,
            )
            .map_err(host_error)?,
        );
        Ok(())
    }

    fn enable(&mut self) -> HostResult<()> {
        self.plugin_mut()?.enable().map_err(host_error)
    }

    fn invoke(&mut self, operation: &str, input: &[u8], callback_id: u64) -> HostResult<Vec<u8>> {
        let input = Value::from_wire(input).map_err(|_| invalid_value())?;
        self.plugin_mut()?
            .invoke(operation, &input, callback_id)
            .map_err(host_error)?
            .to_wire()
            .map_err(|_| invalid_value())
    }

    fn disable(&mut self) -> HostResult<()> {
        self.plugin_mut()?.disable().map_err(host_error)
    }

    fn unload(&mut self) -> HostResult<()> {
        let mut plugin = self.plugin.take().ok_or_else(invalid_state)?;
        plugin.unload().map_err(host_error)
    }
}

impl QuickJsGuestEngine {
    fn plugin_mut(&mut self) -> HostResult<&mut QuickJsPlugin> {
        self.plugin.as_mut().ok_or_else(invalid_state)
    }
}

fn host_error(error: EngineError) -> HostError {
    HostError::new(error.code(), error.to_string())
}

fn invalid_state() -> HostError {
    HostError::new("invalid-state", "QuickJS payload is not loaded")
}

fn invalid_value() -> HostError {
    HostError::new("invalid-value", "Bridge Value is invalid")
}
