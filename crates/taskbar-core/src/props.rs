//! The loosely typed property maps used by both tray protocols.
//!
//! `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu` both describe
//! their state as string-keyed property dictionaries (`a{sv}`) whose contents
//! depend on the application and the library version it was built against.
//! [`PropValue`] is the domain-side decoding of those dictionaries: the
//! daemon converts D-Bus variants into it once, and from then on only typed
//! code is involved.

use std::collections::BTreeMap;

use crate::icon::Pixmap;
use crate::sni::ToolTip;

/// A decoded property value. Anything we do not understand is kept as
/// [`PropValue::Unsupported`] with its signature, so unknown additions to the
/// specs are ignored instead of breaking the tray.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Str(String),
    Bool(bool),
    Int(i64),
    UInt(u64),
    Double(f64),
    Strings(Vec<String>),
    /// Key chords as sent by dbusmenu's `shortcut` property (`aas`).
    StringLists(Vec<Vec<String>>),
    Bytes(Vec<u8>),
    Pixmaps(Vec<Pixmap>),
    ToolTip(Box<ToolTip>),
    Unsupported(String),
}

impl PropValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropValue::Str(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            PropValue::Int(value) => Some(*value),
            PropValue::UInt(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_pixmaps(&self) -> Option<&[Pixmap]> {
        match self {
            PropValue::Pixmaps(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_strings(&self) -> Option<&[String]> {
        match self {
            PropValue::Strings(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_string_lists(&self) -> Option<&[Vec<String>]> {
        match self {
            PropValue::StringLists(value) => Some(value),
            _ => None,
        }
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> Self {
        PropValue::Str(value.to_owned())
    }
}

impl From<String> for PropValue {
    fn from(value: String) -> Self {
        PropValue::Str(value)
    }
}

impl From<bool> for PropValue {
    fn from(value: bool) -> Self {
        PropValue::Bool(value)
    }
}

impl From<i64> for PropValue {
    fn from(value: i64) -> Self {
        PropValue::Int(value)
    }
}

/// A decoded property dictionary.
pub type PropMap = BTreeMap<String, PropValue>;
