//! serde_yaml::Value -> serde_json::Value 的转换(与 115driver 主项目一致)。

pub fn yaml_to_json(v: &serde_yaml::Value) -> serde_json::Value {
    use serde_yaml::Value as Y;
    match v {
        Y::Null => serde_json::Value::Null,
        Y::Bool(b) => serde_json::Value::Bool(*b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Value::Number(serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()))
            } else {
                serde_json::Value::Null
            }
        }
        Y::String(s) => serde_json::Value::String(s.clone()),
        Y::Sequence(a) => serde_json::Value::Array(a.iter().map(yaml_to_json).collect()),
        Y::Mapping(m) => {
            let mut o = serde_json::Map::new();
            for (k, v) in m {
                if let Some(s) = k.as_str() {
                    o.insert(s.to_string(), yaml_to_json(v));
                }
            }
            serde_json::Value::Object(o)
        }
        _ => serde_json::Value::Null,
    }
}
