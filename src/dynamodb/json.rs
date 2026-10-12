//! The single adapter from SDK attribute values to our plain JSON display format.
//! Keep precision, set, and binary conventions here when changing conversion libraries.
use super::{Item, LoadError};
use aws_sdk_dynamodb::types::AttributeValue;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Number, Value};
use std::str::FromStr;

pub(super) fn item_json(item: Item) -> Result<Value, LoadError> {
    let mut object = item
        .into_iter()
        .map(|(name, value)| Ok((name, attribute_json(value)?)))
        .collect::<Result<serde_json::Map<_, _>, LoadError>>()?;
    // GPUI enables preserve_order, so JSON keys follow insertion order.
    // Sort them explicitly to keep the output alphabetical and consistent.
    object.sort_keys();
    Ok(Value::Object(object))
}

pub(super) fn attribute_json(value: AttributeValue) -> Result<Value, LoadError> {
    Ok(match value {
        AttributeValue::S(value) => Value::String(value),
        AttributeValue::N(value) => {
            Value::Number(Number::from_str(&value).map_err(|_| LoadError::Unexpected)?)
        }
        AttributeValue::Bool(value) => Value::Bool(value),
        AttributeValue::Null(_) => Value::Null,
        AttributeValue::B(value) => Value::String(STANDARD.encode(value.as_ref())),
        AttributeValue::L(values) => Value::Array(
            values
                .into_iter()
                .map(attribute_json)
                .collect::<Result<_, _>>()?,
        ),
        AttributeValue::M(item) => item_json(item)?,
        AttributeValue::Ss(values) => Value::Array(values.into_iter().map(Value::String).collect()),
        AttributeValue::Ns(values) => Value::Array(
            values
                .into_iter()
                .map(|value| attribute_json(AttributeValue::N(value)))
                .collect::<Result<_, _>>()?,
        ),
        AttributeValue::Bs(values) => Value::Array(
            values
                .into_iter()
                .map(|value| Value::String(STANDARD.encode(value.as_ref())))
                .collect(),
        ),
        _ => return Err(LoadError::Unexpected),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn object_keys_are_sorted_recursively_without_reordering_arrays() {
        let nested = || {
            AttributeValue::M(HashMap::from([
                ("z".into(), AttributeValue::N("2".into())),
                ("a".into(), AttributeValue::N("1".into())),
            ]))
        };
        let item = HashMap::from([
            ("z".into(), AttributeValue::Bool(true)),
            ("nested".into(), nested()),
            (
                "items".into(),
                AttributeValue::L(vec![
                    AttributeValue::S("zebra".into()),
                    AttributeValue::S("apple".into()),
                    nested(),
                ]),
            ),
            ("a".into(), AttributeValue::Null(true)),
        ]);
        assert_eq!(
            item_json(item).unwrap().to_string(),
            r#"{"a":null,"items":["zebra","apple",{"a":1,"z":2}],"nested":{"a":1,"z":2},"z":true}"#,
        );
    }

    #[test]
    fn plain_json_preserves_numbers_and_nested_types() {
        let value = attribute_json(AttributeValue::M(HashMap::from([
            (
                "n".into(),
                AttributeValue::N("123456789012345678901234567890.12345678".into()),
            ),
            (
                "nested".into(),
                AttributeValue::L(vec![AttributeValue::Bool(true), AttributeValue::Null(true)]),
            ),
            ("binary".into(), AttributeValue::B(vec![72, 105].into())),
            (
                "set".into(),
                AttributeValue::Ns(vec!["1".into(), "2".into()]),
            ),
        ])))
        .unwrap();
        assert_eq!(
            value["n"].to_string(),
            "123456789012345678901234567890.12345678"
        );
        assert_eq!(value["nested"], serde_json::json!([true, null]));
        assert_eq!(value["binary"], "SGk=");
        assert_eq!(value["set"], serde_json::json!([1, 2]));
    }
}
