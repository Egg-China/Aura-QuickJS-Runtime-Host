use crate::{Context, EngineError, EngineResult, QuickJsRuntime};
use aura_bridge_value::{Error, ErrorCode, HandleValue, Value};
use libquickjs_ng_sys as qjs;
use std::collections::HashSet;
use std::ffi::CString;
use std::time::{Duration, Instant};

const VALUE_INIT_TIMEOUT: Duration = Duration::from_secs(1);

const HELPER_SOURCE: &str = r#"(() => {
  const MapCtor = Map;
  const mapSet = Map.prototype.set;
  const mapEntries = Map.prototype.entries;
  const arrayFrom = Array.from;
  const handleData = new WeakMap();
  const errorData = new WeakMap();
  class AuraHandle {
    constructor(objectId, generation, typeName) {
      handleData.set(this, [objectId, generation, typeName]);
      Object.freeze(this);
    }
    get objectId() { return handleData.get(this)[0]; }
    get generation() { return handleData.get(this)[1]; }
    get typeName() { return handleData.get(this)[2]; }
  }
  class AuraError {
    constructor(code) {
      errorData.set(this, [code]);
      Object.freeze(this);
    }
    get code() { return errorData.get(this)[0]; }
  }
  Object.freeze(AuraHandle.prototype);
  Object.freeze(AuraHandle);
  Object.freeze(AuraError.prototype);
  Object.freeze(AuraError);
  return Object.freeze({
    newMap: () => new MapCtor(),
    setMap: (map, key, value) => mapSet.call(map, key, value),
    mapEntries: map => arrayFrom(mapEntries.call(map)),
    newHandle: (objectId, generation, typeName) => new AuraHandle(objectId, generation, typeName),
    readHandle: value => handleData.get(value),
    newError: code => new AuraError(code),
    readError: value => errorData.get(value),
    AuraHandle,
    AuraError
  });
})()"#;

pub(crate) struct ValueIntrinsics {
    helper: qjs::JSValue,
}

impl ValueIntrinsics {
    pub(crate) fn raw(&self) -> qjs::JSValue {
        self.helper
    }
}

impl QuickJsRuntime {
    pub(crate) fn initialize_value_intrinsics(&mut self) -> EngineResult<()> {
        let deadline = Instant::now()
            .checked_add(VALUE_INIT_TIMEOUT)
            .ok_or_else(|| EngineError::new("deadline-exceeded"))?;
        let helper = self.run_with_deadline(deadline, initialize)?;
        self.value_intrinsics = Some(ValueIntrinsics { helper });
        Ok(())
    }

    /// Converts a Bridge value to JavaScript and back through the real engine.
    pub fn round_trip_value(&mut self, value: &Value) -> EngineResult<Value> {
        let helper = self.value_helper()?;
        let deadline = Instant::now()
            .checked_add(VALUE_INIT_TIMEOUT)
            .ok_or_else(|| EngineError::new("deadline-exceeded"))?;
        self.run_with_deadline(deadline, |context| {
            let raw = to_raw(context, helper, value)?;
            let converted = from_raw(context, helper, raw);
            // SAFETY: raw belongs to this context and is released exactly once.
            unsafe { qjs::JS_FreeValue(context.raw(), raw) };
            converted
        })
    }

    /// Returns the JavaScript identity used for a converted Bridge value.
    pub fn javascript_kind(&mut self, value: &Value) -> EngineResult<String> {
        let helper = self.value_helper()?;
        let deadline = Instant::now()
            .checked_add(VALUE_INIT_TIMEOUT)
            .ok_or_else(|| EngineError::new("deadline-exceeded"))?;
        self.run_with_deadline(deadline, |context| {
            let raw = to_raw(context, helper, value)?;
            // SAFETY: raw is live and belongs to this QuickJS build.
            let kind = unsafe {
                if qjs::JS_Ext_IsBigInt(raw) {
                    "bigint"
                } else if qjs::JS_GetTypedArrayType(raw)
                    == qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8
                {
                    "Uint8Array"
                } else if qjs::JS_IsMap(raw) {
                    "Map"
                } else if qjs::JS_IsArray(raw) {
                    "Array"
                } else {
                    "other"
                }
            };
            // SAFETY: raw belongs to this context and is released exactly once.
            unsafe { qjs::JS_FreeValue(context.raw(), raw) };
            Ok(kind.to_owned())
        })
    }

    /// Evaluates one JavaScript expression and converts its result to a Bridge value.
    pub fn eval_value(&mut self, source: &str, timeout: Duration) -> EngineResult<Value> {
        let helper = self.value_helper()?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| EngineError::new("deadline-exceeded"))?;
        self.run_with_deadline(deadline, |context| {
            let source = CString::new(source).map_err(|_| EngineError::new("invalid-value"))?;
            // SAFETY: Source and filename remain live for the call; result is owned below.
            let raw = unsafe {
                qjs::JS_Eval(
                    context.raw(),
                    source.as_ptr(),
                    source.as_bytes().len(),
                    c"aura:value".as_ptr(),
                    qjs::JS_EVAL_TYPE_GLOBAL as i32,
                )
            };
            // SAFETY: raw is live and belongs to this QuickJS build.
            if unsafe { qjs::JS_Ext_IsException(raw) } {
                // SAFETY: evaluation left a pending exception on this context.
                return Err(unsafe { context.classify_exception("evaluation-failed") });
            }
            let converted = from_raw(context, helper, raw);
            // SAFETY: raw belongs to this context and is released exactly once.
            unsafe { qjs::JS_FreeValue(context.raw(), raw) };
            converted
        })
    }

    pub(crate) fn value_helper(&self) -> EngineResult<qjs::JSValue> {
        self.value_intrinsics
            .as_ref()
            .map(ValueIntrinsics::raw)
            .ok_or_else(|| EngineError::new("runtime-failure"))
    }
}

fn initialize(context: &mut Context<'_>) -> EngineResult<qjs::JSValue> {
    let source = CString::new(HELPER_SOURCE).expect("value helper contains no NUL");
    // SAFETY: Source and filename remain live for the call; result is owned by the runtime.
    let helper = unsafe {
        qjs::JS_Eval(
            context.raw(),
            source.as_ptr(),
            source.as_bytes().len(),
            c"aura:value-intrinsics".as_ptr(),
            qjs::JS_EVAL_TYPE_GLOBAL as i32,
        )
    };
    // SAFETY: helper is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(helper) } {
        // SAFETY: initialization left a pending exception on this context.
        Err(unsafe { context.classify_exception("runtime-failure") })
    } else {
        Ok(helper)
    }
}

pub(crate) fn to_raw(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    value: &Value,
) -> EngineResult<qjs::JSValue> {
    value
        .to_wire()
        .map_err(|_| EngineError::new("invalid-value"))?;
    to_raw_validated(context, helper, value)
}

fn to_raw_validated(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    value: &Value,
) -> EngineResult<qjs::JSValue> {
    // SAFETY: Every constructor is called for the live owned context. Container property setters
    // consume child values, while helper calls borrow their arguments.
    unsafe {
        match value {
            Value::Null => Ok(qjs::JS_Ext_NewSpecialValue(qjs::JS_TAG_NULL, 0)),
            Value::Bool(value) => Ok(qjs::JS_Ext_NewBool(context.raw(), u8::from(*value))),
            Value::Integer(value) => Ok(qjs::JS_NewBigInt64(context.raw(), *value)),
            Value::Float(value) => Ok(qjs::JS_NewNumber(context.raw(), *value)),
            Value::String(value) => Ok(qjs::JS_NewStringLen(
                context.raw(),
                value.as_ptr().cast(),
                value.len(),
            )),
            Value::Bytes(value) => Ok(qjs::JS_NewUint8ArrayCopy(
                context.raw(),
                value.as_ptr(),
                value.len(),
            )),
            Value::Array(values) => {
                let array = qjs::JS_NewArray(context.raw());
                if qjs::JS_Ext_IsException(array) {
                    return Err(context.classify_exception("resource-limit"));
                }
                for (index, value) in values.iter().enumerate() {
                    let child = to_raw_validated(context, helper, value)?;
                    if qjs::JS_SetPropertyUint32(context.raw(), array, index as u32, child) < 0 {
                        qjs::JS_FreeValue(context.raw(), array);
                        return Err(context.classify_exception("resource-limit"));
                    }
                }
                Ok(array)
            }
            Value::Map(entries) => {
                let map = call_helper(context, helper, "newMap", &[])?;
                for (key, value) in entries {
                    let key = qjs::JS_NewStringLen(context.raw(), key.as_ptr().cast(), key.len());
                    let child = to_raw_validated(context, helper, value)?;
                    let result = call_helper(context, helper, "setMap", &[map, key, child]);
                    qjs::JS_FreeValue(context.raw(), key);
                    qjs::JS_FreeValue(context.raw(), child);
                    match result {
                        Ok(result) => qjs::JS_FreeValue(context.raw(), result),
                        Err(error) => {
                            qjs::JS_FreeValue(context.raw(), map);
                            return Err(error);
                        }
                    }
                }
                Ok(map)
            }
            Value::Handle(handle) => {
                let object_id = qjs::JS_NewBigInt64(context.raw(), handle.object_id() as i64);
                let generation = qjs::JS_NewBigInt64(context.raw(), handle.generation() as i64);
                let type_name = qjs::JS_NewStringLen(
                    context.raw(),
                    handle.type_name().as_ptr().cast(),
                    handle.type_name().len(),
                );
                let result = call_helper(
                    context,
                    helper,
                    "newHandle",
                    &[object_id, generation, type_name],
                );
                qjs::JS_FreeValue(context.raw(), object_id);
                qjs::JS_FreeValue(context.raw(), generation);
                qjs::JS_FreeValue(context.raw(), type_name);
                result
            }
            Value::Error(error) => {
                let code = error.code().wire_code();
                let code = qjs::JS_NewStringLen(context.raw(), code.as_ptr().cast(), code.len());
                let result = call_helper(context, helper, "newError", &[code]);
                qjs::JS_FreeValue(context.raw(), code);
                result
            }
        }
    }
}

pub(crate) fn from_raw(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    raw: qjs::JSValue,
) -> EngineResult<Value> {
    let mut visited = HashSet::new();
    let value = from_raw_inner(context, helper, raw, &mut visited)?;
    value
        .to_wire()
        .map_err(|_| EngineError::new("invalid-value"))?;
    Ok(value)
}

fn from_raw_inner(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    raw: qjs::JSValue,
    visited: &mut HashSet<usize>,
) -> EngineResult<Value> {
    // SAFETY: raw is borrowed from the live owned context for all type checks and conversions.
    unsafe {
        if qjs::JS_Ext_IsNull(raw) {
            return Ok(Value::Null);
        }
        if qjs::JS_Ext_IsBool(raw) {
            return Ok(Value::Bool(qjs::JS_Ext_GetBool(raw) != 0));
        }
        if qjs::JS_Ext_IsBigInt(raw) {
            let value = read_string(context, raw)?
                .parse::<i64>()
                .map_err(|_| EngineError::new("invalid-value"))?;
            return Ok(Value::Integer(value));
        }
        if qjs::JS_Ext_IsNumber(raw) {
            let mut value = 0_f64;
            if qjs::JS_ToFloat64(context.raw(), &mut value, raw) < 0 || !value.is_finite() {
                if qjs::JS_HasException(context.raw()) {
                    context.discard_exception();
                }
                return Err(EngineError::new("invalid-value"));
            }
            return Ok(Value::Float(value));
        }
        if qjs::JS_Ext_IsString(raw) {
            return read_string(context, raw).map(Value::String);
        }

        let typed_array = qjs::JS_GetTypedArrayType(raw);
        if typed_array == qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8 {
            let mut length = 0_usize;
            let bytes = qjs::JS_GetUint8Array(context.raw(), &mut length, raw);
            if bytes.is_null() {
                if qjs::JS_HasException(context.raw()) {
                    context.discard_exception();
                }
                return Err(EngineError::new("invalid-value"));
            }
            return Ok(Value::Bytes(
                std::slice::from_raw_parts(bytes, length).to_vec(),
            ));
        }

        if qjs::JS_IsArray(raw) {
            return with_visited(raw, visited, |visited| {
                let mut length = 0_i64;
                if qjs::JS_GetLength(context.raw(), raw, &mut length) < 0
                    || !(0..=1024).contains(&length)
                {
                    if qjs::JS_HasException(context.raw()) {
                        context.discard_exception();
                    }
                    return Err(EngineError::new("invalid-value"));
                }
                let mut values = Vec::with_capacity(length as usize);
                for index in 0..length as u32 {
                    let child = qjs::JS_GetPropertyUint32(context.raw(), raw, index);
                    if qjs::JS_Ext_IsException(child) {
                        return Err(context.classify_exception("invalid-value"));
                    }
                    let converted = from_raw_inner(context, helper, child, visited);
                    qjs::JS_FreeValue(context.raw(), child);
                    values.push(converted?);
                }
                Ok(Value::Array(values))
            });
        }

        if qjs::JS_IsMap(raw) {
            return with_visited(raw, visited, |visited| {
                let entries = call_helper(context, helper, "mapEntries", &[raw])?;
                let converted = map_entries(context, helper, entries, visited);
                qjs::JS_FreeValue(context.raw(), entries);
                converted
            });
        }

        if qjs::JS_Ext_IsObject(raw) {
            if let Some(handle) = read_handle(context, helper, raw)? {
                return Ok(Value::Handle(handle));
            }
            if let Some(error) = read_error(context, helper, raw)? {
                return Ok(Value::Error(error));
            }
        }
    }
    Err(EngineError::new("invalid-value"))
}

fn map_entries(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    entries: qjs::JSValue,
    visited: &mut HashSet<usize>,
) -> EngineResult<Value> {
    // SAFETY: entries was returned by the captured helper as an array.
    unsafe {
        let mut length = 0_i64;
        if !qjs::JS_IsArray(entries)
            || qjs::JS_GetLength(context.raw(), entries, &mut length) < 0
            || !(0..=1024).contains(&length)
        {
            if qjs::JS_HasException(context.raw()) {
                context.discard_exception();
            }
            return Err(EngineError::new("invalid-value"));
        }
        let mut output = Vec::with_capacity(length as usize);
        for index in 0..length as u32 {
            let pair = qjs::JS_GetPropertyUint32(context.raw(), entries, index);
            let mut pair_length = 0_i64;
            if !qjs::JS_IsArray(pair)
                || qjs::JS_GetLength(context.raw(), pair, &mut pair_length) < 0
                || pair_length != 2
            {
                qjs::JS_FreeValue(context.raw(), pair);
                return Err(EngineError::new("invalid-value"));
            }
            let key = qjs::JS_GetPropertyUint32(context.raw(), pair, 0);
            let value = qjs::JS_GetPropertyUint32(context.raw(), pair, 1);
            let converted = (|| {
                if !qjs::JS_Ext_IsString(key) {
                    return Err(EngineError::new("invalid-value"));
                }
                let key = read_string(context, key)?;
                let value = from_raw_inner(context, helper, value, visited)?;
                Ok((key, value))
            })();
            qjs::JS_FreeValue(context.raw(), key);
            qjs::JS_FreeValue(context.raw(), value);
            qjs::JS_FreeValue(context.raw(), pair);
            output.push(converted?);
        }
        Ok(Value::Map(output))
    }
}

fn read_handle(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    raw: qjs::JSValue,
) -> EngineResult<Option<HandleValue>> {
    let data = call_helper(context, helper, "readHandle", &[raw])?;
    // SAFETY: data is a live helper result in this context.
    unsafe {
        if qjs::JS_Ext_IsUndefined(data) {
            qjs::JS_FreeValue(context.raw(), data);
            return Ok(None);
        }
        let object_id = qjs::JS_GetPropertyUint32(context.raw(), data, 0);
        let generation = qjs::JS_GetPropertyUint32(context.raw(), data, 1);
        let type_name = qjs::JS_GetPropertyUint32(context.raw(), data, 2);
        let mut object_id_value = 0_i64;
        let mut generation_value = 0_i64;
        let result = if qjs::JS_ToBigInt64(context.raw(), &mut object_id_value, object_id) < 0
            || qjs::JS_ToBigInt64(context.raw(), &mut generation_value, generation) < 0
            || !qjs::JS_Ext_IsString(type_name)
            || object_id_value <= 0
            || generation_value <= 0
        {
            if qjs::JS_HasException(context.raw()) {
                context.discard_exception();
            }
            Err(EngineError::new("invalid-value"))
        } else {
            HandleValue::new(
                object_id_value as u64,
                generation_value as u64,
                read_string(context, type_name)?,
            )
            .map(Some)
            .map_err(|_| EngineError::new("invalid-value"))
        };
        qjs::JS_FreeValue(context.raw(), object_id);
        qjs::JS_FreeValue(context.raw(), generation);
        qjs::JS_FreeValue(context.raw(), type_name);
        qjs::JS_FreeValue(context.raw(), data);
        result
    }
}

fn read_error(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    raw: qjs::JSValue,
) -> EngineResult<Option<Error>> {
    let data = call_helper(context, helper, "readError", &[raw])?;
    // SAFETY: data is a live helper result in this context.
    unsafe {
        if qjs::JS_Ext_IsUndefined(data) {
            qjs::JS_FreeValue(context.raw(), data);
            return Ok(None);
        }
        let code = qjs::JS_GetPropertyUint32(context.raw(), data, 0);
        let result = if qjs::JS_Ext_IsString(code) {
            parse_error_code(&read_string(context, code)?)
                .map(Error::new)
                .map(Some)
                .ok_or_else(|| EngineError::new("invalid-value"))
        } else {
            Err(EngineError::new("invalid-value"))
        };
        qjs::JS_FreeValue(context.raw(), code);
        qjs::JS_FreeValue(context.raw(), data);
        result
    }
}

fn parse_error_code(code: &str) -> Option<ErrorCode> {
    match code {
        "invalid-argument" => Some(ErrorCode::InvalidArgument),
        "invalid-result" => Some(ErrorCode::InvalidResult),
        "permission-denied" => Some(ErrorCode::PermissionDenied),
        "stale-handle" => Some(ErrorCode::StaleHandle),
        "type-mismatch" => Some(ErrorCode::TypeMismatch),
        "cancelled" => Some(ErrorCode::Cancelled),
        "callback-failed" => Some(ErrorCode::CallbackFailed),
        "unavailable" => Some(ErrorCode::Unavailable),
        "internal" => Some(ErrorCode::Internal),
        _ => None,
    }
}

fn read_string(context: &mut Context<'_>, raw: qjs::JSValue) -> EngineResult<String> {
    let mut length = 0_usize;
    // SAFETY: raw is a live string in this context; returned pointer is freed below.
    let pointer = unsafe { qjs::JS_ToCStringLen2(context.raw(), &mut length, raw, false) };
    if pointer.is_null() {
        // SAFETY: conversion left a pending exception if allocation failed.
        if unsafe { qjs::JS_HasException(context.raw()) } {
            // SAFETY: this context owns the pending exception.
            unsafe { context.discard_exception() };
        }
        return Err(EngineError::new("invalid-value"));
    }
    // SAFETY: QuickJS returned exactly length initialized bytes.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
    let value = std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| EngineError::new("invalid-value"));
    // SAFETY: pointer was returned by JS_ToCStringLen2 for this context.
    unsafe { qjs::JS_FreeCString(context.raw(), pointer) };
    value
}

fn call_helper(
    context: &mut Context<'_>,
    helper: qjs::JSValue,
    name: &'static str,
    arguments: &[qjs::JSValue],
) -> EngineResult<qjs::JSValue> {
    let name = CString::new(name).expect("static helper name contains no NUL");
    // SAFETY: helper belongs to this context, name is NUL-terminated, and arguments remain live.
    let function = unsafe { qjs::JS_GetPropertyStr(context.raw(), helper, name.as_ptr()) };
    // SAFETY: function is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(function) } {
        // SAFETY: property access left a pending exception.
        return Err(unsafe { context.classify_exception("runtime-failure") });
    }
    // SAFETY: JS_Call borrows helper and argument values for the duration of the call.
    let result = unsafe {
        qjs::JS_Call(
            context.raw(),
            function,
            helper,
            arguments.len() as i32,
            arguments.as_ptr().cast_mut(),
        )
    };
    // SAFETY: function belongs to this context and is released exactly once.
    unsafe { qjs::JS_FreeValue(context.raw(), function) };
    // SAFETY: result is live and belongs to this QuickJS build.
    if unsafe { qjs::JS_Ext_IsException(result) } {
        // SAFETY: helper call left a pending exception.
        Err(unsafe { context.classify_exception("invalid-value") })
    } else {
        Ok(result)
    }
}

fn with_visited<T>(
    raw: qjs::JSValue,
    visited: &mut HashSet<usize>,
    call: impl FnOnce(&mut HashSet<usize>) -> EngineResult<T>,
) -> EngineResult<T> {
    // SAFETY: raw was confirmed to be an object before this identity lookup.
    let identity = unsafe { qjs::JS_Ext_GetPtr(raw) } as usize;
    if identity == 0 || !visited.insert(identity) {
        return Err(EngineError::new("invalid-value"));
    }
    let result = call(visited);
    visited.remove(&identity);
    result
}
