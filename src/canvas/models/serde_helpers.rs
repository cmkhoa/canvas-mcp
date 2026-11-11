//! Deserializers for Canvas's irregular fields.
use serde::Deserialize;

/// Wraps a present field (even `null`) in `Some`; with `#[serde(default)]` an absent one is `None`.
pub(super) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Accepts `false` (or null) as `None`, otherwise deserializes `T`.
pub(super) fn false_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(false) | serde_json::Value::Null => Ok(None),
        other => serde_json::from_value(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}
