//! Deserializadores tolerantes: los paneles Xtream mandan números como cadena, como
//! número o simplemente los omiten (ver `docs/XTREAM-API.md`). Ninguno de estos falla
//! por un tipo inesperado; a lo sumo devuelven `None`.

use serde::de::{Deserialize, Deserializer};
use serde_json::Value;

pub fn lenient_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(value_to_i64))
}

pub fn lenient_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?
        .and_then(value_to_i64)
        .and_then(|n| u32::try_from(n).ok()))
}

pub fn lenient_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(value_to_f64))
}

/// Id opcional (p. ej. `category_id` referenciado desde un canal): número, texto, cadena
/// vacía o ausente se tratan todos como "sin categoría".
pub fn lenient_id_opt<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?.and_then(|value| match value {
        Value::String(s) if !s.is_empty() => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }))
}

fn value_to_i64(value: Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn value_to_f64(value: Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    struct Probe {
        #[serde(default, deserialize_with = "super::lenient_u32")]
        n: Option<u32>,
    }

    #[derive(Deserialize)]
    struct ProbeId {
        #[serde(default, deserialize_with = "super::lenient_id_opt")]
        id: Option<String>,
    }

    #[test]
    fn accepts_number_string_null_and_missing() {
        assert_eq!(serde_json::from_value::<Probe>(json!({"n": 5})).unwrap().n, Some(5));
        assert_eq!(serde_json::from_value::<Probe>(json!({"n": "5"})).unwrap().n, Some(5));
        assert_eq!(serde_json::from_value::<Probe>(json!({"n": null})).unwrap().n, None);
        assert_eq!(serde_json::from_value::<Probe>(json!({"n": "abc"})).unwrap().n, None);
        assert_eq!(serde_json::from_value::<Probe>(json!({})).unwrap().n, None);
    }

    #[test]
    fn lenient_id_treats_empty_string_as_none() {
        assert_eq!(
            serde_json::from_value::<ProbeId>(json!({"id": ""})).unwrap().id,
            None
        );
        assert_eq!(
            serde_json::from_value::<ProbeId>(json!({"id": 42})).unwrap().id,
            Some("42".to_string())
        );
        assert_eq!(
            serde_json::from_value::<ProbeId>(json!({"id": "cat-1"})).unwrap().id,
            Some("cat-1".to_string())
        );
    }
}
