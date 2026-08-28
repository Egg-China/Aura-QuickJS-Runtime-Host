use crate::value::{from_raw, parse_error_code, to_raw};
use crate::{Context, EngineError, EngineResult, QuickJsRuntime};
use aura_bridge_value::{Error, ErrorCode, Value};
use aura_runtime_protocol::{BridgeError, BridgeTransport};
use libquickjs_ng_sys as qjs;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::sync::Arc;

const EXPORTS: [&std::ffi::CStr; 3] = [c"bridge", c"AuraHandle", c"AuraError"];

pub(crate) struct BridgeState {
    plugin_id: u64,
    session: u64,
    transport: Arc<dyn BridgeTransport>,
    helper: qjs::JSValue,
    bridge: qjs::JSValue,
    plugin_context: qjs::JSValue,
}

impl BridgeState {
    /// Releases values owned by this state before its context is destroyed.
    ///
    /// # Safety
    ///
    /// `context` must be the live context that created both stored values.
    pub(crate) unsafe fn release(&self, context: *mut qjs::JSContext) {
        // SAFETY: The function contract identifies the owning live context.
        unsafe {
            qjs::JS_FreeValue(context, self.plugin_context);
            qjs::JS_FreeValue(context, self.bridge);
        }
    }
}

impl QuickJsRuntime {
    pub(crate) fn install_bridge(
        &mut self,
        plugin_id: u64,
        session: u64,
        transport: Arc<dyn BridgeTransport>,
    ) -> EngineResult<()> {
        if self.bridge_state.is_some() {
            return Err(EngineError::new("runtime-failure"));
        }
        let plugin_id_value =
            i64::try_from(plugin_id).map_err(|_| EngineError::new("invalid-value"))?;
        let helper = self.value_helper()?;
        let context = self.raw_context();

        // SAFETY: All values are constructed in the live owned context and every property setter
        // consumes exactly one newly-created value.
        let (bridge, plugin_context) = unsafe {
            let bridge = qjs::JS_NewObject(context);
            if qjs::JS_Ext_IsException(bridge) {
                return Err(take_callback_exception(context, "resource-limit"));
            }
            for (name, callback, length) in [
                (c"invoke", Some(bridge_invoke as _), 2),
                (c"retain", Some(bridge_retain as _), 1),
                (c"release", Some(bridge_release as _), 1),
            ] {
                let function = qjs::JS_NewCFunction2(
                    context,
                    callback,
                    name.as_ptr(),
                    length,
                    qjs::JSCFunctionEnum_JS_CFUNC_generic,
                    0,
                );
                if qjs::JS_SetPropertyStr(context, bridge, name.as_ptr(), function) < 0 {
                    qjs::JS_FreeValue(context, bridge);
                    return Err(take_callback_exception(context, "resource-limit"));
                }
            }
            if qjs::JS_FreezeObject(context, bridge) < 0 {
                qjs::JS_FreeValue(context, bridge);
                return Err(take_callback_exception(context, "runtime-failure"));
            }

            let plugin_context = qjs::JS_NewObject(context);
            if qjs::JS_Ext_IsException(plugin_context) {
                qjs::JS_FreeValue(context, bridge);
                return Err(take_callback_exception(context, "resource-limit"));
            }
            let plugin_id = qjs::JS_NewBigInt64(context, plugin_id_value);
            let bridge_reference = qjs::JS_DupValue(context, bridge);
            if qjs::JS_SetPropertyStr(context, plugin_context, c"pluginId".as_ptr(), plugin_id) < 0
                || qjs::JS_SetPropertyStr(
                    context,
                    plugin_context,
                    c"bridge".as_ptr(),
                    bridge_reference,
                ) < 0
                || qjs::JS_FreezeObject(context, plugin_context) < 0
            {
                qjs::JS_FreeValue(context, plugin_context);
                qjs::JS_FreeValue(context, bridge);
                return Err(take_callback_exception(context, "runtime-failure"));
            }
            (bridge, plugin_context)
        };

        let mut state = Box::new(BridgeState {
            plugin_id,
            session,
            transport,
            helper,
            bridge,
            plugin_context,
        });
        // SAFETY: The boxed state has a stable address until runtime drop clears the opaque value.
        unsafe {
            qjs::JS_SetContextOpaque(context, (&mut *state as *mut BridgeState).cast::<c_void>());
        }
        self.bridge_state = Some(state);
        Ok(())
    }

    pub(crate) fn plugin_context(&self) -> EngineResult<qjs::JSValue> {
        self.bridge_state
            .as_ref()
            .map(|state| state.plugin_context)
            .ok_or_else(|| EngineError::new("runtime-failure"))
    }
}

/// Creates the native `aura:runtime` module for the active payload context.
///
/// # Safety
///
/// `context` and `module_name` must be valid callback arguments from QuickJS.
pub(crate) unsafe fn create_module(
    context: *mut qjs::JSContext,
    module_name: *const c_char,
) -> *mut qjs::JSModuleDef {
    if context.is_null() || module_name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: Callback arguments are valid and module_init uses the same live context.
    let module = unsafe { qjs::JS_NewCModule(context, module_name, Some(module_init)) };
    if module.is_null() {
        return ptr::null_mut();
    }
    for name in EXPORTS {
        // SAFETY: module and context were just created together; names are static C strings.
        if unsafe { qjs::JS_AddModuleExport(context, module, name.as_ptr()) } < 0 {
            return ptr::null_mut();
        }
    }
    module
}

/// Initializes the three native module exports.
///
/// # Safety
///
/// QuickJS must call this with the live module and context created by `create_module`.
unsafe extern "C" fn module_init(
    context: *mut qjs::JSContext,
    module: *mut qjs::JSModuleDef,
) -> c_int {
    let Some(state) = (unsafe { state(context) }) else {
        return -1;
    };
    // SAFETY: Stored values belong to this context. Module export setters consume their values.
    unsafe {
        let bridge = qjs::JS_DupValue(context, state.bridge);
        if qjs::JS_SetModuleExport(context, module, c"bridge".as_ptr(), bridge) < 0 {
            return -1;
        }
        for name in [c"AuraHandle", c"AuraError"] {
            let value = qjs::JS_GetPropertyStr(context, state.helper, name.as_ptr());
            if qjs::JS_Ext_IsException(value)
                || qjs::JS_SetModuleExport(context, module, name.as_ptr(), value) < 0
            {
                return -1;
            }
        }
    }
    0
}

/// Performs one nested Bridge invoke and returns a settled promise.
///
/// # Safety
///
/// QuickJS must supply a live context and `argc` accessible values at `argv`.
unsafe extern "C" fn bridge_invoke(
    context: *mut qjs::JSContext,
    _this_value: qjs::JSValue,
    argc: c_int,
    argv: *mut qjs::JSValue,
) -> qjs::JSValue {
    callback_guard(context, || {
        let arguments = unsafe { arguments(argc, argv, 2)? };
        let state = unsafe { state(context) }.ok_or(ErrorCode::Internal)?;
        let mut scoped = unsafe { Context::from_raw(context) }.map_err(|_| ErrorCode::Internal)?;
        let operation = match from_raw(&mut scoped, state.helper, arguments[0]) {
            Ok(Value::String(operation)) => operation,
            _ => return Err(ErrorCode::InvalidArgument),
        };
        let input = from_raw(&mut scoped, state.helper, arguments[1])
            .map_err(|_| ErrorCode::InvalidArgument)?
            .to_wire()
            .map_err(|_| ErrorCode::InvalidArgument)?;
        let output = state
            .transport
            .invoke(state.plugin_id, state.session, &operation, &input)
            .map_err(bridge_error_code)?;
        Value::from_wire(&output).map_err(|_| ErrorCode::InvalidResult)
    })
}

/// Retains one host handle and returns a settled promise.
///
/// # Safety
///
/// QuickJS must supply a live context and `argc` accessible values at `argv`.
unsafe extern "C" fn bridge_retain(
    context: *mut qjs::JSContext,
    _this_value: qjs::JSValue,
    argc: c_int,
    argv: *mut qjs::JSValue,
) -> qjs::JSValue {
    handle_callback(context, argc, argv, true)
}

/// Releases one host handle and returns a settled promise.
///
/// # Safety
///
/// QuickJS must supply a live context and `argc` accessible values at `argv`.
unsafe extern "C" fn bridge_release(
    context: *mut qjs::JSContext,
    _this_value: qjs::JSValue,
    argc: c_int,
    argv: *mut qjs::JSValue,
) -> qjs::JSValue {
    handle_callback(context, argc, argv, false)
}

fn handle_callback(
    context: *mut qjs::JSContext,
    argc: c_int,
    argv: *mut qjs::JSValue,
    retain: bool,
) -> qjs::JSValue {
    callback_guard(context, || {
        let arguments = unsafe { arguments(argc, argv, 1)? };
        let state = unsafe { state(context) }.ok_or(ErrorCode::Internal)?;
        let mut scoped = unsafe { Context::from_raw(context) }.map_err(|_| ErrorCode::Internal)?;
        let Value::Handle(handle) = from_raw(&mut scoped, state.helper, arguments[0])
            .map_err(|_| ErrorCode::InvalidArgument)?
        else {
            return Err(ErrorCode::InvalidArgument);
        };
        let result = if retain {
            state
                .transport
                .retain_handle(state.session, handle.object_id(), handle.generation())
        } else {
            state
                .transport
                .release_handle(state.session, handle.object_id(), handle.generation())
        };
        result.map_err(bridge_error_code)?;
        Ok(Value::Null)
    })
}

fn callback_guard(
    context: *mut qjs::JSContext,
    call: impl FnOnce() -> Result<Value, ErrorCode>,
) -> qjs::JSValue {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
        .unwrap_or(Err(ErrorCode::Internal));
    let Some(state) = (unsafe { state(context) }) else {
        // SAFETY: QuickJS supplied the callback context; a missing state is an internal error.
        return unsafe { qjs::JS_ThrowInternalError(context, c"Bridge unavailable".as_ptr()) };
    };
    // SAFETY: context is live and exclusively executing this callback.
    let Ok(mut scoped) = (unsafe { Context::from_raw(context) }) else {
        // SAFETY: QuickJS supplied the callback context.
        return unsafe { qjs::JS_ThrowInternalError(context, c"Bridge unavailable".as_ptr()) };
    };
    if unsafe { qjs::JS_HasException(context) } {
        // SAFETY: This callback owns the pending exception.
        unsafe { scoped.discard_exception() };
    }
    let (value, rejected) = match result {
        Ok(value) => (value, false),
        Err(code) => (Value::Error(Error::new(code)), true),
    };
    let raw = match to_raw(&mut scoped, state.helper, &value) {
        Ok(raw) => raw,
        Err(_) => {
            // SAFETY: QuickJS supplied the callback context.
            return unsafe {
                qjs::JS_ThrowInternalError(context, c"Bridge value failure".as_ptr())
            };
        }
    };
    // SAFETY: JS_NewSettledPromise borrows raw and returns an owned promise.
    let promise = unsafe { qjs::JS_NewSettledPromise(context, rejected, raw) };
    // SAFETY: raw remains owned by this callback and is released after promise creation.
    unsafe { qjs::JS_FreeValue(context, raw) };
    promise
}

unsafe fn arguments<'a>(
    argc: c_int,
    argv: *mut qjs::JSValue,
    expected: usize,
) -> Result<&'a [qjs::JSValue], ErrorCode> {
    if argc != expected as c_int || (expected != 0 && argv.is_null()) {
        return Err(ErrorCode::InvalidArgument);
    }
    // SAFETY: The function contract requires argc accessible values at argv.
    Ok(unsafe { std::slice::from_raw_parts(argv, expected) })
}

unsafe fn state<'a>(context: *mut qjs::JSContext) -> Option<&'a BridgeState> {
    if context.is_null() {
        return None;
    }
    // SAFETY: install_bridge registers a BridgeState pointer for this context until runtime drop.
    unsafe {
        qjs::JS_GetContextOpaque(context)
            .cast::<BridgeState>()
            .as_ref()
    }
}

fn bridge_error_code(error: BridgeError) -> ErrorCode {
    match error {
        BridgeError::Callback(code) => parse_error_code(&code).unwrap_or(ErrorCode::Internal),
        BridgeError::Protocol(_) => ErrorCode::Internal,
    }
}

fn take_callback_exception(context: *mut qjs::JSContext, code: &'static str) -> EngineError {
    // SAFETY: context is the live owned context and the preceding operation left an exception.
    unsafe {
        match Context::from_raw(context) {
            Ok(scoped) => scoped.classify_exception(code),
            Err(error) => error,
        }
    }
}
