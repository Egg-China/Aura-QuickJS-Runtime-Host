use crate::{EngineError, EngineResult, QuickJsRuntime};
use libquickjs_ng_sys as qjs;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::ptr;

const MAX_MODULE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct ModuleLoaderState {
    root: PathBuf,
    canonical_root: PathBuf,
    error: Option<EngineError>,
}

impl ModuleLoaderState {
    pub(crate) fn new(root: &Path) -> EngineResult<Self> {
        let canonical_root =
            fs::canonicalize(root).map_err(|_| EngineError::new("invalid-module"))?;
        if !canonical_root.is_dir() {
            return Err(EngineError::new("invalid-module"));
        }
        Ok(Self {
            root: root.to_path_buf(),
            canonical_root,
            error: None,
        })
    }

    pub(crate) fn root_name(module: &Path) -> EngineResult<String> {
        let mut parts = Vec::new();
        for component in module.components() {
            match component {
                Component::Normal(value) => parts.push(
                    value
                        .to_str()
                        .ok_or_else(|| EngineError::new("invalid-module"))?,
                ),
                _ => return Err(EngineError::new("path-escape")),
            }
        }
        validate_normalized_name(&parts.join("/"))
    }

    pub(crate) fn read(&self, name: &str) -> EngineResult<String> {
        let name = validate_normalized_name(name)?;
        let path = self
            .root
            .join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        let canonical = fs::canonicalize(&path).map_err(|_| EngineError::new("invalid-module"))?;
        if !canonical.starts_with(&self.canonical_root) {
            return Err(EngineError::new("path-escape"));
        }
        let metadata = fs::metadata(&canonical).map_err(|_| EngineError::new("invalid-module"))?;
        if !metadata.is_file() || metadata.len() > MAX_MODULE_BYTES {
            return Err(EngineError::new("invalid-module"));
        }
        let bytes = fs::read(canonical).map_err(|_| EngineError::new("invalid-module"))?;
        let source = String::from_utf8(bytes).map_err(|_| EngineError::new("invalid-module"))?;
        if source.contains('\0') {
            return Err(EngineError::new("invalid-module"));
        }
        Ok(source)
    }

    fn record(&mut self, error: EngineError) {
        self.error = Some(error);
    }

    pub(crate) fn take_error(&mut self) -> Option<EngineError> {
        self.error.take()
    }
}

impl QuickJsRuntime {
    pub(crate) fn install_module_loader(&mut self, root: &Path) -> EngineResult<()> {
        let mut loader = Box::new(ModuleLoaderState::new(root)?);
        let pointer = (&mut *loader as *mut ModuleLoaderState).cast::<c_void>();
        // SAFETY: loader has a stable boxed address and remains owned by self until after the
        // QuickJS runtime is destroyed. Both callbacks catch panics before crossing the C ABI.
        unsafe {
            qjs::JS_SetModuleLoaderFunc(
                self.raw_runtime(),
                Some(normalize_callback),
                Some(load_callback),
                pointer,
            );
        }
        self.module_loader = Some(loader);
        Ok(())
    }

    pub(crate) fn take_module_error(&mut self) -> Option<EngineError> {
        self.module_loader
            .as_mut()
            .and_then(|state| state.take_error())
    }

    pub(crate) fn module_source(&self, name: &str) -> EngineResult<String> {
        self.module_loader
            .as_ref()
            .ok_or_else(|| EngineError::new("runtime-failure"))?
            .read(name)
    }
}

fn validate_normalized_name(name: &str) -> EngineResult<String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains(':')
        || name.contains('\0')
        || !name.ends_with(".mjs")
    {
        return Err(EngineError::new("invalid-module"));
    }
    let mut count = 0_usize;
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            return Err(EngineError::new("invalid-module"));
        }
        if part == ".." {
            return Err(EngineError::new("path-escape"));
        }
        count += 1;
    }
    if count == 0 {
        Err(EngineError::new("invalid-module"))
    } else {
        Ok(name.to_owned())
    }
}

fn normalize_name(base: &str, specifier: &str) -> EngineResult<String> {
    if specifier == "aura:runtime" {
        return Ok(specifier.to_owned());
    }
    if (!specifier.starts_with("./") && !specifier.starts_with("../"))
        || specifier.contains('\\')
        || specifier.contains(':')
        || specifier.contains('\0')
    {
        return Err(EngineError::new("invalid-module"));
    }

    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for part in specifier.split('/') {
        match part {
            "" => return Err(EngineError::new("invalid-module")),
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(EngineError::new("path-escape"));
                }
            }
            value => parts.push(value),
        }
    }
    validate_normalized_name(&parts.join("/"))
}

/// Normalizes a relative module specifier into one package-relative slash path.
///
/// # Safety
///
/// QuickJS must pass valid NUL-terminated names and the live loader pointer registered above.
unsafe extern "C" fn normalize_callback(
    context: *mut qjs::JSContext,
    base: *const c_char,
    specifier: *const c_char,
    opaque: *mut c_void,
) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() || base.is_null() || specifier.is_null() || opaque.is_null() {
            return Err(EngineError::new("runtime-failure"));
        }
        // SAFETY: Pointers were checked and the callback contract guarantees their validity.
        let base = unsafe { CStr::from_ptr(base) }
            .to_str()
            .map_err(|_| EngineError::new("invalid-module"))?;
        // SAFETY: Pointers were checked and the callback contract guarantees their validity.
        let specifier = unsafe { CStr::from_ptr(specifier) }
            .to_str()
            .map_err(|_| EngineError::new("invalid-module"))?;
        let normalized = normalize_name(base, specifier)?;
        let normalized =
            CString::new(normalized).map_err(|_| EngineError::new("invalid-module"))?;
        // SAFETY: context is live for the callback and QuickJS owns memory returned by js_malloc.
        let output = unsafe { qjs::js_malloc(context, normalized.as_bytes_with_nul().len()) };
        if output.is_null() {
            return Err(EngineError::new("resource-limit"));
        }
        // SAFETY: output is a distinct allocation of exactly this length.
        unsafe {
            ptr::copy_nonoverlapping(
                normalized.as_ptr().cast::<u8>(),
                output.cast::<u8>(),
                normalized.as_bytes_with_nul().len(),
            );
        }
        Ok(output.cast::<c_char>())
    }));

    match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            // SAFETY: opaque is checked by the closure or the null check below prevents access.
            if !opaque.is_null() {
                unsafe { &mut *opaque.cast::<ModuleLoaderState>() }.record(error);
            }
            ptr::null_mut()
        }
        Err(_) => {
            if !opaque.is_null() {
                // SAFETY: The callback contract supplies the registered loader pointer.
                unsafe { &mut *opaque.cast::<ModuleLoaderState>() }
                    .record(EngineError::new("runtime-failure"));
            }
            ptr::null_mut()
        }
    }
}

/// Loads and compiles one normalized module source.
///
/// # Safety
///
/// QuickJS must pass a valid context, NUL-terminated name, and live registered loader pointer.
unsafe extern "C" fn load_callback(
    context: *mut qjs::JSContext,
    module_name: *const c_char,
    opaque: *mut c_void,
) -> *mut qjs::JSModuleDef {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() || module_name.is_null() || opaque.is_null() {
            return Err(EngineError::new("runtime-failure"));
        }
        // SAFETY: Pointers were checked and the callback contract guarantees their validity.
        let name = unsafe { CStr::from_ptr(module_name) }
            .to_str()
            .map_err(|_| EngineError::new("invalid-module"))?;
        // SAFETY: opaque is the exclusively accessed loader allocation during this callback.
        let state = unsafe { &mut *opaque.cast::<ModuleLoaderState>() };
        if name == "aura:runtime" {
            return Err(EngineError::new("invalid-module"));
        }
        let source = state.read(name)?;
        let source = CString::new(source).map_err(|_| EngineError::new("invalid-module"))?;
        let filename = CString::new(name).map_err(|_| EngineError::new("invalid-module"))?;
        // SAFETY: All pointers remain live for the call; compile-only returns an owned JSValue.
        let value = unsafe {
            qjs::JS_Eval(
                context,
                source.as_ptr(),
                source.as_bytes().len(),
                filename.as_ptr(),
                (qjs::JS_EVAL_TYPE_MODULE | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32,
            )
        };
        // SAFETY: value is live and belongs to this QuickJS build.
        if unsafe { qjs::JS_Ext_IsException(value) } {
            state.record(EngineError::new("invalid-module"));
            return Ok(ptr::null_mut());
        }
        // SAFETY: A successful compile-only module value contains a JSModuleDef pointer.
        let module = unsafe { qjs::JS_Ext_GetPtr(value) }.cast::<qjs::JSModuleDef>();
        // SAFETY: QuickJS retains the compiled module; release this temporary JSValue reference.
        unsafe { qjs::JS_FreeValue(context, value) };
        Ok(module)
    }));

    match result {
        Ok(Ok(module)) => module,
        Ok(Err(error)) => {
            if !opaque.is_null() {
                // SAFETY: The callback contract supplies the registered loader pointer.
                unsafe { &mut *opaque.cast::<ModuleLoaderState>() }.record(error);
            }
            if !context.is_null() {
                // SAFETY: context is live and the constant contains no formatting directives.
                unsafe { qjs::JS_ThrowInternalError(context, c"module rejected".as_ptr()) };
            }
            ptr::null_mut()
        }
        Err(_) => {
            if !opaque.is_null() {
                // SAFETY: The callback contract supplies the registered loader pointer.
                unsafe { &mut *opaque.cast::<ModuleLoaderState>() }
                    .record(EngineError::new("runtime-failure"));
            }
            ptr::null_mut()
        }
    }
}
