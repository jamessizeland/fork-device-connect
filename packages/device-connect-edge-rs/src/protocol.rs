// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::types::{DeviceCapabilities, DeviceIdentity, DeviceStatus, now_rfc3339};

pub const JSONRPC_VERSION: &str = "2.0";
pub const ERR_PARSE: i64 = -32700;
pub const ERR_INVALID_REQUEST: i64 = -32600;
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;
pub const ERR_INTERNAL: i64 = -32603;
pub const ERR_DRIVER: i64 = -32000;

pub mod subjects {
    pub fn command(tenant: &str, device_id: &str) -> String {
        format!("device-connect.{tenant}.{device_id}.cmd")
    }

    pub fn registry(tenant: &str) -> String {
        format!("device-connect.{tenant}.registry")
    }

    pub fn heartbeat(tenant: &str, device_id: &str) -> String {
        format!("device-connect.{tenant}.{device_id}.heartbeat")
    }

    pub fn presence(tenant: &str, device_id: &str) -> String {
        format!("device-connect.{tenant}.{device_id}.presence")
    }

    pub fn presence_wildcard(tenant: &str) -> String {
        format!("device-connect.{tenant}.*.presence")
    }

    pub fn event(tenant: &str, device_id: &str, event_name: &str) -> String {
        format!("device-connect.{tenant}.{device_id}.event.{event_name}")
    }

    pub fn broadcast(tenant: &str) -> String {
        format!("device-connect.{tenant}.broadcast")
    }

    pub fn async_reply(tenant: &str, device_id: &str, correlation_id: &str) -> String {
        format!("device-connect.{tenant}.{device_id}.event.async_reply.{correlation_id}")
    }

    pub fn discovery_probe(tenant: &str) -> String {
        format!("device-connect.{tenant}.discovery.probe")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: format!("rs-{}", Uuid::new_v4().simple()),
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcErrorBody {
    pub code: i64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success {
        jsonrpc: String,
        id: String,
        result: Value,
    },
    Error {
        jsonrpc: String,
        id: Option<String>,
        error: JsonRpcErrorBody,
    },
}

impl JsonRpcResponse {
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self::Success {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: id.into(),
            result,
        }
    }

    pub fn error(id: Option<String>, code: i64, message: impl Into<String>) -> Self {
        Self::Error {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            error: JsonRpcErrorBody {
                code,
                message: message.into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegistrationParams {
    pub device_id: String,
    pub device_ttl: u64,
    pub capabilities: DeviceCapabilities,
    pub identity: DeviceIdentity,
    pub status: DeviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub device_id: String,
    pub ts: f64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub device_id: String,
    pub tenant: String,
    pub capabilities: DeviceCapabilities,
    pub identity: DeviceIdentity,
    pub status: DeviceStatus,
    pub ts: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departing: Option<bool>,
}

impl Presence {
    pub fn departing(device_id: impl Into<String>, tenant: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            tenant: tenant.into(),
            capabilities: DeviceCapabilities::default(),
            identity: DeviceIdentity::default(),
            status: DeviceStatus::default(),
            ts: unix_time_f64(),
            departing: Some(true),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl EventNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BroadcastEnvelope {
    pub correlation_id: String,
    pub function: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_at: Option<f64>,
    #[serde(default = "default_on_late")]
    pub on_late: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BroadcastReply {
    pub correlation_id: String,
    pub device_id: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    pub actually_fired_at: f64,
}

pub fn build_registration_request(params: RegistrationParams) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id: format!("reg-{}-{}", params.device_id, Uuid::new_v4().simple()),
        method: "registerDevice".to_string(),
        params: serde_json::to_value(params).unwrap_or_else(|_| json!({})),
    }
}

pub fn unix_time_f64() -> f64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs_f64()
}

pub fn event_payload_with_defaults(mut payload: Value) -> Value {
    let Value::Object(ref mut obj) = payload else {
        return json!({
            "value": payload,
            "event_id": Uuid::new_v4().simple().to_string()[..8].to_string(),
            "ts": now_rfc3339(),
        });
    };
    obj.entry("event_id".to_string())
        .or_insert_with(|| json!(Uuid::new_v4().simple().to_string()[..8].to_string()));
    obj.entry("ts".to_string())
        .or_insert_with(|| json!(now_rfc3339()));
    payload
}

fn default_on_late() -> String {
    "skip".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_match_python_conventions() {
        assert_eq!(
            subjects::command("default", "sensor-001"),
            "device-connect.default.sensor-001.cmd"
        );
        assert_eq!(
            subjects::event("default", "sensor-001", "reading"),
            "device-connect.default.sensor-001.event.reading"
        );
        assert_eq!(
            subjects::presence_wildcard("alpha"),
            "device-connect.alpha.*.presence"
        );
    }

    #[test]
    fn json_rpc_shapes_match_python() {
        let ok = JsonRpcResponse::success("7", json!({"temperature": 22.5}));
        assert_eq!(
            serde_json::to_value(ok).unwrap(),
            json!({"jsonrpc": "2.0", "id": "7", "result": {"temperature": 22.5}})
        );
        let err = JsonRpcResponse::error(
            Some("7".to_string()),
            ERR_METHOD_NOT_FOUND,
            "Unknown method: nope",
        );
        assert_eq!(
            serde_json::to_value(err).unwrap()["error"]["code"],
            ERR_METHOD_NOT_FOUND
        );
    }

    #[test]
    fn event_payload_gets_python_defaults() {
        let payload = event_payload_with_defaults(json!({"value": 1}));
        assert!(payload.get("event_id").is_some());
        assert!(payload.get("ts").is_some());
        assert_eq!(payload["value"], 1);
    }
}
