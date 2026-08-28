use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub req: String,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(flatten)]
    pub params: serde_json::Map<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
pub enum Reply {
    Ok(Value),
    Err(ErrorBody),
}

impl Reply {
    pub fn ok(v: Value) -> Self {
        Reply::Ok(v)
    }
    pub fn err(code: &str, message: impl Into<String>) -> Self {
        Reply::Err(ErrorBody { code: code.into(), message: message.into() })
    }
    pub fn into_json(self, id: Value) -> Value {
        match self {
            Reply::Ok(v) => serde_json::json!({"reply": id, "ok": true, "result": v}),
            Reply::Err(e) => serde_json::json!({"reply": id, "ok": false, "error": e}),
        }
    }
}
