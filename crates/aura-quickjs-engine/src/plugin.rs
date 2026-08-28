use crate::module_loader::ModuleLoaderState;
use crate::value::{from_raw, to_raw};
use crate::{Context, EngineError, EngineResult, Limits, QuickJsRuntime};
use aura_bridge_value::Value;
use aura_runtime_protocol::BridgeTransport;
use libquickjs_ng_sys as qjs;
use std::ffi::CString;
use std::fmt;
use std::path::Path;
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const EXPORTS: [&str; 5] = ["load", "enable", "invoke", "disable", "unload"];

/// Owns one loaded JavaScript payload and its isolated QuickJS runtime.
pub struct QuickJsPlugin {
    runtime: QuickJsRuntime,
    namespace: Option<qjs::JSValue>,
    plugin_id: u64,
    session: u64,
}

impl QuickJsPlugin {
    /// Loads a module graph, validates lifecycle exports, and calls `load`.
    pub fn load(
        root: &Path,
        module: &Path,
        plugin_id: u64,
        session: u64,
        bridge: Arc<dyn BridgeTransport>,
    ) -> EngineResult<Self> {
        let module_name = ModuleLoaderState::root_name(module)?;
        let mut runtime = QuickJsRuntime::new(Limits::default())?;
        runtime.install_bridge(plugin_id, session, bridge)?;
        runtime.install_module_loader(root)?;
        let source = runtime.module_source(&module_name)?;
        let result = runtime.run_with_deadline(deadline()?, |context| {
            compile_and_evaluate(context, &source, &module_name)
        });
        let namespace = match result {
            Ok(namespace) => namespace,
            Err(error) => return Err(runtime.take_module_error().unwrap_or(error)),
        };
        let mut plugin = Self {
            runtime,
            namespace: Some(namespace),
            plugin_id,
            session,
        };
        plugin.validate_exports()?;
        plugin.call_load()?;
        Ok(plugin)
    }

    /// Calls the payload's `enable` lifecycle function.
    pub fn enable(&mut self) -> EngineResult<()> {
        self.call("enable")
    }

    /// Calls the payload's `disable` lifecycle function.
    pub fn disable(&mut self) -> EngineResult<()> {
        self.call("disable")
    }

    /// Calls `invoke` with lossless Bridge values and returns its converted result.
    pub fn invoke(
        &mut self,
        operation: &str,
        input: &Value,
        callback_id: u64,
    ) -> EngineResult<Value> {
        let callback_id =
            i64::try_from(callback_id).map_err(|_| EngineError::new("invalid-value"))?;
        let namespace = self
            .namespace
            .ok_or_else(|| EngineError::new("runtime-failure"))?;
        let helper = self.runtime.value_helper()?;
        self.runtime.run_with_deadline(deadline()?, |context| {
            // SAFETY: Creates one string in the live owned context.
            let operation_value = unsafe {
                qjs::JS_NewStringLen(context.raw(), operation.as_ptr().cast(), operation.len())
            };
            let input_value = match to_raw(context, helper, input) {
                Ok(value) => value,
                Err(error) => {
                    // SAFETY: operation_value belongs to this context and is released once.
                    unsafe { qjs::JS_FreeValue(context.raw(), operation_value) };
                    return Err(error);
                }
            };
            // SAFETY: Creates a signed BigInt in this live context.
            let callback_value = unsafe { qjs::JS_NewBigInt64(context.raw(), callback_id) };
            let result = call_export_result(
                context,
                namespace,
                "invoke",
                &[operation_value, input_value, callback_value],
            );
            // SAFETY: JS_Call borrowed these arguments; all remain owned here and are released.
            unsafe {
                qjs::JS_FreeValue(context.raw(), operation_value);
                qjs::JS_FreeValue(context.raw(), input_value);
                qjs::JS_FreeValue(context.raw(), callback_value);
            }
            let result = result?;
            let converted = from_raw(context, helper, result);
            // SAFETY: result belongs to this context and is released exactly once.
            unsafe { qjs::JS_FreeValue(context.raw(), result) };
            converted
        })
    }

    /// Calls the payload's `unload` lifecycle function and releases its namespace.
    pub fn unload(&mut self) -> EngineResult<()> {
        self.call("unload")?;
        self.release_namespace();
        Ok(())
    }

    fn validate_exports(&mut self) -> EngineResult<()> {
        let namespace = self
            .namespace
            .ok_or_else(|| EngineError::new("runtime-failure"))?;
        self.runtime.run_with_deadline(deadline()?, |context| {
            for name in EXPORTS {
                let name = CString::new(name).expect("static export name has no NUL");
                // SAFETY: namespace belongs to this context and name is NUL-terminated.
                let value =
                    unsafe { qjs::JS_GetPropertyStr(context.raw(), namespace, name.as_ptr()) };
                // SAFETY: value is live and belongs to this QuickJS build.
                let exception = unsafe { qjs::JS_Ext_IsException(value) };
                // SAFETY: value was returned by this context and is released exactly once.
                let callable = !exception && unsafe { qjs::JS_IsFunction(context.raw(), value) };
                // SAFETY: value belongs to this context and has not been released yet.
                unsafe { qjs::JS_FreeValue(context.raw(), value) };
                if exception {
                    // SAFETY: the property access left a pending exception on this context.
                    unsafe { context.classify_exception("invalid-export") };
                    return Err(EngineError::new("invalid-export"));
                }
                if !callable {
                    return Err(EngineError::new("invalid-export"));
                }
            }
            Ok(())
        })
    }

    fn call(&mut self, name: &'static str) -> EngineResult<()> {
        let namespace = self
            .namespace
            .ok_or_else(|| EngineError::new("runtime-failure"))?;
        self.runtime
            .run_with_deadline(deadline()?, |context| call_export(context, namespace, name))
    }

    fn call_load(&mut self) -> EngineResult<()> {
        let namespace = self
            .namespace
            .ok_or_else(|| EngineError::new("runtime-failure"))?;
        let plugin_context = self.runtime.plugin_context()?;
        self.runtime.run_with_deadline(deadline()?, |context| {
            let result = call_export_result(context, namespace, "load", &[plugin_context])?;
            // SAFETY: result belongs to this context and is released exactly once.
            unsafe { qjs::JS_FreeValue(context.raw(), result) };
            Ok(())
        })
    }

    fn release_namespace(&mut self) {
        if let Some(namespace) = self.namespace.take() {
            // SAFETY: namespace belongs to the still-live context and is released once.
            unsafe { qjs::JS_FreeValue(self.runtime.raw_context(), namespace) };
        }
    }
}

impl Drop for QuickJsPlugin {
    fn drop(&mut self) {
        self.release_namespace();
    }
}

impl fmt::Debug for QuickJsPlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuickJsPlugin")
            .field("plugin_id", &self.plugin_id)
            .field("session", &self.session)
            .field("loaded", &self.namespace.is_some())
            .field("bridge", &"configured")
            .finish()
    }
}

fn compile_and_evaluate(
    context: &mut Context<'_>,
    source: &str,
    module_name: &str,
) -> EngineResult<qjs::JSValue> {
    let source = CString::new(source).map_err(|_| EngineError::new("invalid-module"))?;
    let module_name = CString::new(module_name).map_err(|_| EngineError::new("invalid-module"))?;
    // SAFETY: Source and filename remain live for the call; compile-only returns an owned value.
    let module = unsafe {
        qjs::JS_Eval(
            context.raw(),
            source.as_ptr(),
            source.as_bytes().len(),
            module_name.as_ptr(),
            (qjs::JS_EVAL_TYPE_MODULE | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32,
        )
    };
    // SAFETY: module is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(module) } {
        // SAFETY: compilation left a pending exception on this context.
        return Err(unsafe { context.classify_exception("invalid-module") });
    }
    // SAFETY: A successful compile-only module value contains a live module definition.
    let definition = unsafe { qjs::JS_Ext_GetPtr(module) }.cast::<qjs::JSModuleDef>();
    if definition.is_null() {
        // SAFETY: module belongs to this context and has not been released.
        unsafe { qjs::JS_FreeValue(context.raw(), module) };
        return Err(EngineError::new("runtime-failure"));
    }
    // SAFETY: module is a compiled module owned by this context.
    let evaluated = unsafe { qjs::JS_EvalFunction(context.raw(), module) };
    // SAFETY: definition remains registered in the runtime after module evaluation.
    let namespace = unsafe { qjs::JS_GetModuleNamespace(context.raw(), definition) };
    // SAFETY: namespace is a fresh value returned by this context.
    if unsafe { qjs::JS_Ext_IsException(namespace) } {
        // SAFETY: evaluated belongs to this context and is no longer needed.
        unsafe { qjs::JS_FreeValue(context.raw(), evaluated) };
        // SAFETY: namespace lookup left a pending exception.
        return Err(unsafe { context.classify_exception("invalid-module") });
    }
    if let Err(error) = settle(context, evaluated, "invalid-module") {
        // SAFETY: namespace belongs to this context and must be released on failure.
        unsafe { qjs::JS_FreeValue(context.raw(), namespace) };
        return Err(error);
    }
    Ok(namespace)
}

fn call_export(
    context: &mut Context<'_>,
    namespace: qjs::JSValue,
    name: &'static str,
) -> EngineResult<()> {
    let result = call_export_result(context, namespace, name, &[])?;
    // SAFETY: result belongs to this context and is released exactly once.
    unsafe { qjs::JS_FreeValue(context.raw(), result) };
    Ok(())
}

fn call_export_result(
    context: &mut Context<'_>,
    namespace: qjs::JSValue,
    name: &'static str,
    arguments: &[qjs::JSValue],
) -> EngineResult<qjs::JSValue> {
    let name = CString::new(name).expect("static export name has no NUL");
    // SAFETY: namespace belongs to this context and name is NUL-terminated.
    let function = unsafe { qjs::JS_GetPropertyStr(context.raw(), namespace, name.as_ptr()) };
    // SAFETY: function is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(function) } {
        // SAFETY: property access left a pending exception.
        return Err(unsafe { context.classify_exception("invalid-export") });
    }
    // SAFETY: Creates the immediate undefined value for the owned context.
    let this_value = unsafe { qjs::JS_Ext_NewSpecialValue(qjs::JS_TAG_UNDEFINED, 0) };
    // SAFETY: function, this_value, and every borrowed argument belong to this context.
    let result = unsafe {
        qjs::JS_Call(
            context.raw(),
            function,
            this_value,
            arguments.len() as i32,
            arguments.as_ptr().cast_mut(),
        )
    };
    // SAFETY: function and this_value have not been released and are no longer needed.
    unsafe {
        qjs::JS_FreeValue(context.raw(), function);
        qjs::JS_FreeValue(context.raw(), this_value);
    }
    settle(context, result, "guest-exception")
}

fn settle(
    context: &mut Context<'_>,
    value: qjs::JSValue,
    rejection_code: &'static str,
) -> EngineResult<qjs::JSValue> {
    // SAFETY: value is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(value) } {
        // SAFETY: the prior call left a pending exception.
        return Err(unsafe { context.classify_exception(rejection_code) });
    }
    // SAFETY: value is live and belongs to this QuickJS build.
    if !unsafe { qjs::JS_IsPromise(value) } {
        return Ok(value);
    }

    loop {
        // SAFETY: value is a live promise owned by this context.
        let state = unsafe { qjs::JS_PromiseState(context.raw(), value) };
        match state {
            qjs::JSPromiseStateEnum_JS_PROMISE_FULFILLED => {
                // SAFETY: value is a settled promise and result returns an owned reference.
                let result = unsafe { qjs::JS_PromiseResult(context.raw(), value) };
                // SAFETY: the promise is no longer needed and is released once.
                unsafe { qjs::JS_FreeValue(context.raw(), value) };
                return Ok(result);
            }
            qjs::JSPromiseStateEnum_JS_PROMISE_REJECTED => {
                // SAFETY: value is a rejected promise and result returns an owned reference.
                let reason = unsafe { qjs::JS_PromiseResult(context.raw(), value) };
                // SAFETY: both values belong to this context and are released once.
                unsafe {
                    qjs::JS_FreeValue(context.raw(), reason);
                    qjs::JS_FreeValue(context.raw(), value);
                }
                return Err(EngineError::new(rejection_code));
            }
            qjs::JSPromiseStateEnum_JS_PROMISE_PENDING => {}
            _ => {
                // SAFETY: value belongs to this context and is released once.
                unsafe { qjs::JS_FreeValue(context.raw(), value) };
                return Err(EngineError::new("runtime-failure"));
            }
        }

        if context.deadline_expired() {
            // SAFETY: value belongs to this context and is released once.
            unsafe { qjs::JS_FreeValue(context.raw(), value) };
            return Err(EngineError::new("deadline-exceeded"));
        }
        let mut job_context = ptr::null_mut();
        // SAFETY: runtime pointer is obtained from the live owned context; output pointer is valid.
        let status = unsafe {
            qjs::JS_ExecutePendingJob(qjs::JS_GetRuntime(context.raw()), &mut job_context)
        };
        if status < 0 {
            // SAFETY: value belongs to this context and is released once.
            unsafe { qjs::JS_FreeValue(context.raw(), value) };
            // SAFETY: the failed job left an exception on its context, which is this runtime's
            // single context for payload jobs.
            return Err(unsafe { context.classify_exception(rejection_code) });
        }
        if status == 0 {
            std::thread::yield_now();
        }
    }
}

fn deadline() -> EngineResult<Instant> {
    Instant::now()
        .checked_add(CALL_TIMEOUT)
        .ok_or_else(|| EngineError::new("deadline-exceeded"))
}
