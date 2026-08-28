use crate::module_loader::ModuleLoaderState;
use crate::value::ValueIntrinsics;
use libquickjs_ng_sys as qjs;
use std::cell::Cell;
use std::ffi::{CStr, CString, c_void};
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

const EVAL_FILENAME: &CStr = c"aura:payload";

/// Resource limits applied to one QuickJS runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum memory managed by QuickJS, in bytes.
    pub memory_bytes: usize,
    /// Maximum native stack consumed by QuickJS, in bytes.
    pub stack_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 128 * 1024 * 1024,
            stack_bytes: 1024 * 1024,
        }
    }
}

/// Stable failure returned by the QuickJS boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineError {
    code: &'static str,
}

impl EngineError {
    /// Returns the stable machine-readable failure code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn new(code: &'static str) -> Self {
        Self { code }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for EngineError {}

/// Result produced by the QuickJS engine boundary.
pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Debug)]
struct InterruptState {
    deadline: Cell<Option<Instant>>,
    interrupted: Cell<bool>,
}

/// Owns one bounded QuickJS runtime and its single context.
pub struct QuickJsRuntime {
    runtime: NonNull<qjs::JSRuntime>,
    context: NonNull<qjs::JSContext>,
    interrupt: Box<InterruptState>,
    pub(crate) module_loader: Option<Box<ModuleLoaderState>>,
    pub(crate) value_intrinsics: Option<ValueIntrinsics>,
    _not_send_or_sync: std::marker::PhantomData<Rc<()>>,
}

/// Provides scoped access to a context owned by a `QuickJsRuntime`.
pub struct Context<'runtime> {
    context: NonNull<qjs::JSContext>,
    interrupt: &'runtime InterruptState,
    _exclusive: std::marker::PhantomData<&'runtime mut qjs::JSContext>,
}

impl QuickJsRuntime {
    /// Creates a runtime and context with fixed heap and stack limits.
    pub fn new(limits: Limits) -> Result<Self, EngineError> {
        if limits.memory_bytes == 0 || limits.stack_bytes == 0 {
            return Err(EngineError::new("resource-limit"));
        }

        let mut interrupt = Box::new(InterruptState {
            deadline: Cell::new(None),
            interrupted: Cell::new(false),
        });

        // SAFETY: QuickJS constructors and setters are called in their required order. The boxed
        // interrupt state has a stable address and outlives the runtime handler registration.
        unsafe {
            let runtime = NonNull::new(qjs::JS_NewRuntime())
                .ok_or_else(|| EngineError::new("resource-limit"))?;
            qjs::JS_SetMemoryLimit(runtime.as_ptr(), limits.memory_bytes);
            qjs::JS_SetMaxStackSize(runtime.as_ptr(), limits.stack_bytes);
            qjs::JS_SetInterruptHandler(
                runtime.as_ptr(),
                Some(interrupt_handler),
                (&mut *interrupt as *mut InterruptState).cast::<c_void>(),
            );

            let Some(context) = NonNull::new(qjs::JS_NewContext(runtime.as_ptr())) else {
                qjs::JS_FreeRuntime(runtime.as_ptr());
                return Err(EngineError::new("resource-limit"));
            };

            let mut runtime = Self {
                runtime,
                context,
                interrupt,
                module_loader: None,
                value_intrinsics: None,
                _not_send_or_sync: std::marker::PhantomData,
            };
            runtime.initialize_value_intrinsics()?;
            Ok(runtime)
        }
    }

    /// Evaluates a script and converts its result to a signed integer.
    pub fn eval_int(&mut self, source: &str, timeout: Duration) -> Result<i64, EngineError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| EngineError::new("deadline-exceeded"))?;
        self.run_with_deadline(deadline, |context| context.eval_int(source))
    }

    /// Runs one context call under an absolute monotonic deadline.
    pub fn run_with_deadline<T>(
        &mut self,
        deadline: Instant,
        call: impl FnOnce(&mut Context<'_>) -> EngineResult<T>,
    ) -> EngineResult<T> {
        self.interrupt.interrupted.set(false);
        self.interrupt.deadline.set(Some(deadline));
        let mut context = Context {
            context: self.context,
            interrupt: &self.interrupt,
            _exclusive: std::marker::PhantomData,
        };
        let result = call(&mut context);
        self.interrupt.deadline.set(None);
        result
    }

    pub(crate) fn raw_runtime(&self) -> *mut qjs::JSRuntime {
        self.runtime.as_ptr()
    }

    pub(crate) fn raw_context(&self) -> *mut qjs::JSContext {
        self.context.as_ptr()
    }
}

impl Context<'_> {
    /// Evaluates a script and converts its result to a signed integer.
    pub fn eval_int(&mut self, source: &str) -> EngineResult<i64> {
        let source = CString::new(source).map_err(|_| EngineError::new("evaluation-failed"))?;

        // SAFETY: The context is owned by self, source and filename pointers remain valid for the
        // entire call, and the returned value is released exactly once below.
        let value = unsafe {
            qjs::JS_Eval(
                self.context.as_ptr(),
                source.as_ptr(),
                source.as_bytes().len(),
                EVAL_FILENAME.as_ptr(),
                qjs::JS_EVAL_TYPE_GLOBAL as i32,
            )
        };

        // SAFETY: value was returned by this QuickJS build and remains live.
        if unsafe { qjs::JS_Ext_IsException(value) } {
            // SAFETY: The active exception belongs to this context and the returned exception
            // value is consumed by classify_exception.
            return Err(unsafe { self.classify_exception("evaluation-failed") });
        }

        let mut result = 0_i64;
        // SAFETY: value belongs to this context. Conversion borrows it, then it is released once.
        let conversion = unsafe { qjs::JS_ToInt64(self.context.as_ptr(), &mut result, value) };
        // SAFETY: value was returned by JS_Eval for this context and has not been freed yet.
        unsafe { qjs::JS_FreeValue(self.context.as_ptr(), value) };

        if conversion < 0 {
            // SAFETY: JS_ToInt64 raised an exception in this context; consume it before returning.
            unsafe { self.discard_exception() };
            Err(EngineError::new("invalid-result"))
        } else {
            Ok(result)
        }
    }

    /// Consumes and classifies the current QuickJS exception.
    ///
    /// # Safety
    ///
    /// The context must have a pending exception and no other code may access it concurrently.
    pub(crate) unsafe fn classify_exception(&self, default_code: &'static str) -> EngineError {
        if self.interrupt.interrupted.get() {
            // SAFETY: The caller guarantees a pending exception on this owned context.
            unsafe { self.discard_exception() };
            return EngineError::new("deadline-exceeded");
        }

        // SAFETY: The caller guarantees a pending exception on this owned context.
        let exception = unsafe { qjs::JS_GetException(self.context.as_ptr()) };
        // SAFETY: exception belongs to this context and remains alive until after conversion.
        let text = unsafe {
            let pointer = qjs::JS_ToCStringLen2(
                self.context.as_ptr(),
                std::ptr::null_mut(),
                exception,
                false,
            );
            if pointer.is_null() {
                None
            } else {
                let message = CStr::from_ptr(pointer).to_string_lossy().into_owned();
                qjs::JS_FreeCString(self.context.as_ptr(), pointer);
                Some(message)
            }
        };
        // SAFETY: exception belongs to this context and has not been released yet.
        unsafe { qjs::JS_FreeValue(self.context.as_ptr(), exception) };

        match text.as_deref() {
            Some(message)
                if message.contains("out of memory")
                    || message.contains("stack overflow")
                    || message.contains("Maximum call stack size exceeded") =>
            {
                EngineError::new("resource-limit")
            }
            _ => EngineError::new(default_code),
        }
    }

    /// Releases the current exception without exposing its text or stack.
    ///
    /// # Safety
    ///
    /// The context must have a pending exception and no other code may access it concurrently.
    pub(crate) unsafe fn discard_exception(&self) {
        // SAFETY: The caller guarantees a pending exception on this owned context.
        let exception = unsafe { qjs::JS_GetException(self.context.as_ptr()) };
        // SAFETY: exception was just removed from this context and must be released once.
        unsafe { qjs::JS_FreeValue(self.context.as_ptr(), exception) };
    }

    pub(crate) fn raw(&self) -> *mut qjs::JSContext {
        self.context.as_ptr()
    }

    pub(crate) fn deadline_expired(&self) -> bool {
        self.interrupt
            .deadline
            .get()
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl Drop for QuickJsRuntime {
    fn drop(&mut self) {
        self.interrupt.deadline.set(None);
        if let Some(intrinsics) = self.value_intrinsics.take() {
            // SAFETY: The helper value belongs to the still-live owned context.
            unsafe { qjs::JS_FreeValue(self.context.as_ptr(), intrinsics.raw()) };
        }
        // SAFETY: Both pointers were created together and are exclusively owned. QuickJS requires
        // all contexts to be freed before their runtime. The handler state remains alive until the
        // runtime has been destroyed.
        unsafe {
            qjs::JS_FreeContext(self.context.as_ptr());
            qjs::JS_FreeRuntime(self.runtime.as_ptr());
        }
    }
}

/// Interrupts QuickJS once the active monotonic deadline has elapsed.
///
/// # Safety
///
/// `opaque` must point to the live `InterruptState` registered by `QuickJsRuntime::new`.
unsafe extern "C" fn interrupt_handler(
    _runtime: *mut qjs::JSRuntime,
    opaque: *mut c_void,
) -> std::ffi::c_int {
    if opaque.is_null() {
        return 1;
    }

    // SAFETY: The function contract requires opaque to be our live InterruptState allocation.
    let state = unsafe { &*opaque.cast::<InterruptState>() };
    let expired = state
        .deadline
        .get()
        .is_some_and(|deadline| Instant::now() >= deadline);
    if expired {
        state.interrupted.set(true);
        1
    } else {
        0
    }
}
