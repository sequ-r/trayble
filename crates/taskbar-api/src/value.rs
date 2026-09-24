//! Decoding of D-Bus variants into [`PropValue`].
//!
//! Both tray protocols hand out `a{sv}` property bags whose contents depend
//! on the toolkit and version an application was built with. This is the one
//! place where those variants are interpreted; everything downstream sees
//! typed values and never fails on a shape it does not know.

use std::collections::HashMap;

use taskbar_core::icon::Pixmap;
use taskbar_core::props::{PropMap, PropValue};
use taskbar_core::sni::ToolTip;
use zvariant::{Array, Dict, OwnedValue, Structure, Value};

/// Decode a property dictionary that arrived as a typed `a{sv}`.
pub fn to_prop_map(properties: &HashMap<String, OwnedValue>) -> PropMap {
    properties
        .iter()
        .map(|(name, value)| (name.clone(), to_prop_value(value)))
        .collect()
}

/// Decode a property dictionary that is still a [`Value::Dict`], as found
/// inside the recursive `com.canonical.dbusmenu` layouts.
pub fn dict_to_prop_map(dict: &Dict<'_, '_>) -> PropMap {
    dict.iter()
        .filter_map(|(key, value)| match key {
            Value::Str(name) => Some((name.as_str().to_owned(), to_prop_value(value))),
            _ => None,
        })
        .collect()
}

/// Decode a single variant.
pub fn to_prop_value(value: &Value<'_>) -> PropValue {
    match value {
        Value::Bool(inner) => PropValue::Bool(*inner),
        Value::U8(inner) => PropValue::UInt(*inner as u64),
        Value::I16(inner) => PropValue::Int(*inner as i64),
        Value::U16(inner) => PropValue::UInt(*inner as u64),
        Value::I32(inner) => PropValue::Int(*inner as i64),
        Value::U32(inner) => PropValue::UInt(*inner as u64),
        Value::I64(inner) => PropValue::Int(*inner),
        Value::U64(inner) => PropValue::UInt(*inner),
        Value::F64(inner) => PropValue::Double(*inner),
        Value::Str(inner) => PropValue::Str(inner.as_str().to_owned()),
        Value::ObjectPath(inner) => PropValue::Str(inner.as_str().to_owned()),
        Value::Signature(inner) => PropValue::Unsupported(inner.to_string()),
        // Variants wrapped one level deeper are unwrapped.
        Value::Value(inner) => to_prop_value(inner),
        Value::Array(inner) => to_prop_array(inner),
        Value::Structure(inner) => to_prop_structure(inner),
        other => PropValue::Unsupported(format!("{other:?}")),
    }
}

/// Arrays are decoded by their element signature, which is how the two
/// protocols spell their payload types:
///
/// * `ay` — raw bytes, e.g. `icon-data` of a dbusmenu entry,
/// * `as` — a list of strings,
/// * `aas` — key chords like `[["Control", "Q"]]`,
/// * `a(iiay)` — the icon pixmaps of the StatusNotifierItem spec.
fn to_prop_array(array: &Array<'_>) -> PropValue {
    let elements = || array.inner().iter().map(to_prop_value);

    match array.element_signature().to_string().as_str() {
        "y" => PropValue::Bytes(
            elements()
                .filter_map(|value| match value {
                    PropValue::UInt(byte) => u8::try_from(byte).ok(),
                    _ => None,
                })
                .collect(),
        ),
        "s" | "o" => PropValue::Strings(
            elements()
                .filter_map(|value| match value {
                    PropValue::Str(text) => Some(text),
                    _ => None,
                })
                .collect(),
        ),
        "as" => PropValue::StringLists(
            elements()
                .filter_map(|value| match value {
                    PropValue::Strings(list) => Some(list),
                    _ => None,
                })
                .collect(),
        ),
        "(iiay)" => PropValue::Pixmaps(
            elements()
                .filter_map(|value| match value {
                    PropValue::Pixmaps(mut list) if list.len() == 1 => Some(list.remove(0)),
                    _ => None,
                })
                .collect(),
        ),
        other => PropValue::Unsupported(other.to_owned()),
    }
}

fn to_prop_structure(structure: &Structure<'_>) -> PropValue {
    let fields = structure.fields();

    // `(iiay)` — one icon pixmap of the StatusNotifierItem spec.
    if fields.len() == 3 {
        if let (Value::I32(width), Value::I32(height)) = (&fields[0], &fields[1]) {
            if let PropValue::Bytes(data) = to_prop_value(&fields[2]) {
                return PropValue::Pixmaps(vec![Pixmap::new(*width, *height, data)]);
            }
        }
    }

    // `(sa(iiay)ss)` — the tooltip of the StatusNotifierItem spec.
    if fields.len() == 4 {
        if let (
            PropValue::Str(icon_name),
            PropValue::Pixmaps(icon_pixmaps),
            PropValue::Str(title),
            PropValue::Str(description),
        ) = (
            to_prop_value(&fields[0]),
            to_prop_value(&fields[1]),
            to_prop_value(&fields[2]),
            to_prop_value(&fields[3]),
        ) {
            return PropValue::ToolTip(Box::new(ToolTip {
                icon_name,
                icon_pixmaps,
                title,
                description,
            }));
        }
    }

    PropValue::Unsupported(structure.signature().to_string())
}

/// The `data` argument of a `com.canonical.dbusmenu` `Event` call for plain
/// clicks, matching what the reference implementations send.
pub fn empty_event_data() -> Value<'static> {
    Value::U8(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::Value;

    /// A wire accurate `a(iiay)` array, i.e. one `IconPixmap` property.
    fn pixmap_list(width: i32, height: i32) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(width, height, vec![0u8; (width * height) as usize * 4])]
    }

    fn owned(text: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(text)).expect("owned value")
    }

    #[test]
    fn scalars_decode_to_typed_values() {
        assert_eq!(to_prop_value(&Value::from(true)), PropValue::Bool(true));
        assert_eq!(to_prop_value(&Value::from("hi")), PropValue::Str("hi".into()));
        assert_eq!(to_prop_value(&Value::from(7u32)), PropValue::UInt(7));
        assert_eq!(to_prop_value(&Value::from(-2i32)), PropValue::Int(-2));
    }

    #[test]
    fn nested_variants_unwrap() {
        assert_eq!(
            to_prop_value(&Value::Value(Box::new(Value::from("deep")))),
            PropValue::Str("deep".into())
        );
    }

    #[test]
    fn byte_arrays_decode() {
        let array = Value::Array(Array::from(vec![1u8, 2, 3]));
        assert_eq!(to_prop_value(&array), PropValue::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn string_arrays_decode() {
        let array = Value::Array(
            Array::from(vec!["Control".to_owned(), "Q".to_owned()]),
        );
        assert_eq!(
            to_prop_value(&array),
            PropValue::Strings(vec!["Control".into(), "Q".into()])
        );
    }

    #[test]
    fn shortcut_arrays_decode_to_key_chords() {
        let chords = vec![
            vec!["Control".to_owned(), "Q".to_owned()],
            vec!["Alt".to_owned(), "X".to_owned()],
        ];
        let array = Value::Array(Array::from(chords.clone()));
        assert_eq!(to_prop_value(&array), PropValue::StringLists(chords));
    }

    #[test]
    fn pixmap_arrays_decode_to_pixmaps() {
        let mut list = pixmap_list(2, 2);
        list.extend(pixmap_list(4, 4));
        let array = Value::Array(Array::from(list));
        match to_prop_value(&array) {
            PropValue::Pixmaps(pixmaps) => {
                assert_eq!(pixmaps.len(), 2);
                assert_eq!((pixmaps[0].width, pixmaps[0].height), (2, 2));
                assert!(pixmaps[1].is_valid());
            }
            other => panic!("expected pixmaps, got {other:?}"),
        }
    }

    #[test]
    fn tooltip_structures_decode() {
        let tooltip =
            Structure::from(("icon", pixmap_list(2, 2), "Title", "Description"));

        match to_prop_value(&Value::Structure(tooltip)) {
            PropValue::ToolTip(tooltip) => {
                assert_eq!(tooltip.title, "Title");
                assert_eq!(tooltip.description, "Description");
                assert_eq!(tooltip.icon_pixmaps.len(), 1);
            }
            other => panic!("expected a tooltip, got {other:?}"),
        }
    }

    #[test]
    fn unknown_shapes_are_kept_as_unsupported() {
        let value = Value::Array(Array::from(vec![1.5f64]));
        assert!(matches!(to_prop_value(&value), PropValue::Unsupported(_)));
    }

    #[test]
    fn property_maps_decode_by_name() {
        let mut properties = HashMap::new();
        properties.insert("Id".to_owned(), owned("demo"));
        properties.insert("ItemIsMenu".to_owned(), OwnedValue::from(true));

        let map = to_prop_map(&properties);
        assert_eq!(map["Id"], PropValue::Str("demo".into()));
        assert_eq!(map["ItemIsMenu"], PropValue::Bool(true));
    }
}
