use aura_bridge_value::Value;

#[test]
fn integer_has_the_frozen_bridge_value_encoding() {
    let bytes = Value::Integer(42).to_wire().expect("encode integer");
    assert_eq!(bytes, [0x92, 0x02, 0xd3, 0, 0, 0, 0, 0, 0, 0, 42]);
    assert_eq!(
        Value::from_wire(&bytes).expect("decode integer"),
        Value::Integer(42)
    );
}
