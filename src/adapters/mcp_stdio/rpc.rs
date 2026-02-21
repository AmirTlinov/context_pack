use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct RpcRequest {
    #[allow(dead_code)]
    pub(super) jsonrpc: Option<String>,
    pub(super) id: Option<Value>,
    pub(super) method: String,
    pub(super) params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub(super) struct RpcEnvelope {
    pub(super) jsonrpc: &'static str,
    pub(super) id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub(super) struct RpcError {
    pub(super) code: i64,
    pub(super) message: String,
}

impl RpcEnvelope {
    pub(super) fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub(super) fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}
