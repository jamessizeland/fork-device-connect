// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use crate::errors::{DeviceConnectError, Result};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LabelValue {
    One(String),
    Many(Vec<String>),
}

pub type Labels = BTreeMap<String, LabelValue>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "empty_params_schema")]
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Labels>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl FunctionDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            parameters: empty_params_schema(),
            labels: None,
            tags: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn parameters(mut self, parameters: Value) -> Self {
        self.parameters = parameters;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Labels>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl EventDef {
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        validate_event_name(&name)?;
        Ok(Self {
            name,
            description: String::new(),
            payload_schema: None,
            labels: None,
            tags: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceCapabilities {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub functions: Vec<FunctionDef>,
    #[serde(default)]
    pub events: Vec<EventDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Labels>,
}

impl Default for DeviceCapabilities {
    fn default() -> Self {
        Self {
            description: String::new(),
            functions: Vec::new(),
            events: Vec::new(),
            labels: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commissioning_comment: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceStatus {
    #[serde(default = "now_rfc3339")]
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default = "default_availability")]
    pub availability: String,
    #[serde(default)]
    pub busy_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,
    #[serde(default = "default_online")]
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_state: Option<String>,
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self {
            ts: now_rfc3339(),
            location: None,
            availability: default_availability(),
            busy_score: 0.0,
            battery: None,
            online: true,
            error_state: None,
        }
    }
}

pub fn empty_params_schema() -> Value {
    json!({"type": "object", "properties": {}, "required": []})
}

pub fn validate_device_id(device_id: &str) -> Result<()> {
    let mut chars = device_id.chars();
    let Some(first) = chars.next() else {
        return Err(DeviceConnectError::InvalidDeviceId(device_id.to_string()));
    };
    if !first.is_ascii_alphanumeric() || device_id.len() > 255 {
        return Err(DeviceConnectError::InvalidDeviceId(device_id.to_string()));
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        Ok(())
    } else {
        Err(DeviceConnectError::InvalidDeviceId(device_id.to_string()))
    }
}

pub fn validate_event_name(name: &str) -> Result<()> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(())
    } else {
        Err(DeviceConnectError::InvalidEventName(name.to_string()))
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn default_availability() -> String {
    "idle".to_string()
}

fn default_online() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_event_names_like_python() {
        assert!(validate_event_name("reading_changed").is_ok());
        assert!(validate_event_name("motion-1").is_ok());
        assert!(validate_event_name("event/motion").is_err());
    }

    #[test]
    fn validates_device_ids_like_python() {
        assert!(validate_device_id("sensor-001").is_ok());
        assert!(validate_device_id("robot.1_a").is_ok());
        assert!(validate_device_id("-bad").is_err());
        assert!(validate_device_id("bad space").is_err());
    }

    #[test]
    fn function_def_serializes_python_shape() {
        let def = FunctionDef::new("get_reading");
        let value = serde_json::to_value(def).unwrap();
        assert_eq!(value["parameters"], empty_params_schema());
        assert_eq!(value["tags"], json!([]));
    }
}
