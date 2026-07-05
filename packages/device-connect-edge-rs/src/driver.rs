// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::Result;
use crate::protocol::{JsonRpcRequest, subjects};
use crate::transport::TransportRef;
use crate::types::{DeviceCapabilities, DeviceIdentity, DeviceStatus, EventDef, FunctionDef};

#[async_trait]
pub trait DeviceDriver: Send + Sync + 'static {
    fn definition(&self) -> DeviceDefinition;

    fn identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            device_type: Some(self.definition().device_type),
            ..DeviceIdentity::default()
        }
    }

    fn status(&self) -> DeviceStatus {
        DeviceStatus::default()
    }

    async fn connect(&mut self, _ctx: &DeviceContext) -> Result<()> {
        Ok(())
    }

    async fn disconnect(&mut self, _ctx: &DeviceContext) -> Result<()> {
        Ok(())
    }

    async fn handle_rpc(
        &mut self,
        function: &str,
        params: Value,
        ctx: &DeviceContext,
    ) -> Result<Value>;
}

#[derive(Clone, Debug)]
pub struct DeviceDefinition {
    pub device_type: String,
    pub capabilities: DeviceCapabilities,
}

impl DeviceDefinition {
    pub fn builder(device_type: impl Into<String>) -> DeviceDefinitionBuilder {
        DeviceDefinitionBuilder {
            device_type: device_type.into(),
            description: String::new(),
            functions: Vec::new(),
            events: Vec::new(),
        }
    }
}

pub struct DeviceDefinitionBuilder {
    device_type: String,
    description: String,
    functions: Vec<FunctionDef>,
    events: Vec<EventDef>,
}

impl DeviceDefinitionBuilder {
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn function(mut self, function: FunctionDef) -> Self {
        self.functions.push(function);
        self
    }

    pub fn event(mut self, event: EventDef) -> Self {
        self.events.push(event);
        self
    }

    pub fn build(self) -> DeviceDefinition {
        DeviceDefinition {
            device_type: self.device_type,
            capabilities: DeviceCapabilities {
                description: self.description,
                functions: self.functions,
                events: self.events,
                labels: None,
            },
        }
    }
}

#[derive(Clone)]
pub struct DeviceContext {
    pub(crate) tenant: String,
    pub(crate) device_id: String,
    pub(crate) transport: TransportRef,
}

impl DeviceContext {
    pub(crate) fn new(tenant: String, device_id: String, transport: TransportRef) -> Self {
        Self {
            tenant,
            device_id,
            transport,
        }
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn transport(&self) -> TransportRef {
        self.transport.clone()
    }

    pub async fn emit(&self, event_name: &str, payload: Value) -> Result<()> {
        crate::types::validate_event_name(event_name)?;
        let notification = crate::protocol::EventNotification::new(
            event_name,
            crate::protocol::event_payload_with_defaults(payload),
        );
        let data = serde_json::to_vec(&notification)?;
        self.transport
            .publish(
                &subjects::event(&self.tenant, &self.device_id, event_name),
                data,
            )
            .await
    }

    pub async fn invoke_remote(
        &self,
        device_id: &str,
        function: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let mut req = JsonRpcRequest::new(function, params);
        if let Some(obj) = req.params.as_object_mut() {
            obj.insert(
                "_dc_meta".to_string(),
                serde_json::json!({"source_device": self.device_id}),
            );
        }
        let reply = self
            .transport
            .request(
                &subjects::command(&self.tenant, device_id),
                serde_json::to_vec(&req)?,
                timeout,
            )
            .await?;
        Ok(serde_json::from_slice(&reply)?)
    }
}
