use aura_bridge_value::{Error, ErrorCode, HandleValue, Value};
use aura_quickjs_engine::{Limits, QuickJsRuntime};
use std::time::Duration;

#[test]
fn round_trips_every_bridge_value_tag() {
    let values = [
        Value::Null,
        Value::Bool(true),
        Value::Integer(i64::MAX),
        Value::Float(1.25),
        Value::String("Aura".to_owned()),
        Value::Bytes(vec![0, 1, 255]),
        Value::Array(vec![Value::Null, Value::Integer(-7)]),
        Value::Map(vec![("first".to_owned(), Value::Bool(false))]),
        Value::Handle(HandleValue::new(11, 13, "launcher.profile").expect("valid handle")),
        Value::Error(Error::new(ErrorCode::PermissionDenied)),
    ];
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    for value in values {
        assert_eq!(
            runtime.round_trip_value(&value).expect("round-trip value"),
            value
        );
    }
}

#[test]
fn maps_integer_to_bigint_bytes_to_uint8array_and_map_to_map() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    assert_eq!(
        runtime
            .javascript_kind(&Value::Integer(42))
            .expect("inspect integer"),
        "bigint"
    );
    assert_eq!(
        runtime
            .javascript_kind(&Value::Bytes(vec![1, 2]))
            .expect("inspect bytes"),
        "Uint8Array"
    );
    assert_eq!(
        runtime
            .javascript_kind(&Value::Map(vec![("x".to_owned(), Value::Null)]))
            .expect("inspect map"),
        "Map"
    );
}

#[test]
fn decodes_javascript_containers_without_losing_order() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    assert_eq!(
        runtime
            .eval_value(
                "new Map([['first', 1n], ['second', new Uint8Array([2, 3])]])",
                Duration::from_secs(1),
            )
            .expect("decode map"),
        Value::Map(vec![
            ("first".to_owned(), Value::Integer(1)),
            ("second".to_owned(), Value::Bytes(vec![2, 3])),
        ])
    );
}

#[test]
fn rejects_plain_objects_cycles_and_non_finite_numbers() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    for source in [
        "({ x: 1 })",
        "(() => { const value = []; value.push(value); return value; })()",
        "NaN",
        "Infinity",
        "undefined",
    ] {
        let error = runtime
            .eval_value(source, Duration::from_secs(1))
            .expect_err("invalid JavaScript value must fail");
        assert_eq!(error.code(), "invalid-value", "source: {source}");
    }
}

#[test]
fn rejects_out_of_range_bigints_non_string_keys_and_wrong_byte_views() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    for source in [
        "9223372036854775808n",
        "new Map([[1, null]])",
        "new Int8Array([1, 2])",
        "(() => { const value = new Map(); value.set('self', value); return value; })()",
    ] {
        let error = runtime
            .eval_value(source, Duration::from_secs(1))
            .expect_err("invalid JavaScript value must fail");
        assert_eq!(error.code(), "invalid-value", "source: {source}");
    }
}
