// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use futures::FutureExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, info, warn};

use crate::discovery::{PresenceAnnouncer, PresenceCollector};
use crate::driver::{DeviceContext, DeviceDriver};
use crate::errors::{DeviceConnectError, Result};
use crate::protocol::{
    BroadcastEnvelope, BroadcastReply, ERR_DRIVER, ERR_METHOD_NOT_FOUND, EventNotification,
    Heartbeat, JsonRpcRequest, JsonRpcResponse, RegistrationParams, build_registration_request,
    subjects, unix_time_f64,
};
use crate::transport::{MessageHandler, Subscription, TransportRef};
use crate::transports::zenoh::ZenohTransport;
use crate::types::{validate_device_id, validate_event_name};

pub struct DeviceRuntime<D: DeviceDriver> {
    driver: Arc<Mutex<D>>,
    device_id: String,
    tenant: String,
    ttl: u64,
    heartbeat_interval: Duration,
    allow_insecure: bool,
    transport: TransportRef,
    subscriptions: Vec<Box<dyn Subscription>>,
    tasks: Vec<JoinHandle<()>>,
    presence_announcer: Option<Arc<PresenceAnnouncer>>,
    presence_collector: Option<Arc<PresenceCollector>>,
}

impl<D: DeviceDriver> DeviceRuntime<D> {
    pub fn builder(driver: D) -> DeviceRuntimeBuilder<D> {
        DeviceRuntimeBuilder::new(driver)
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    pub async fn start(&mut self) -> Result<()> {
        validate_device_id(&self.device_id)?;
        if self.allow_insecure {
            warn!("running in insecure Device Connect mode; do not use this in production");
        }

        self.transport.connect().await?;

        let ctx = self.context();
        self.driver.lock().await.connect(&ctx).await?;

        self.subscribe_commands().await?;
        self.subscribe_broadcasts().await?;
        self.start_presence().await?;
        self.start_heartbeat();

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        if let Some(announcer) = &self.presence_announcer {
            announcer.stop().await?;
        }
        if let Some(collector) = &self.presence_collector {
            collector.stop().await?;
        }
        for sub in self.subscriptions.drain(..) {
            sub.unsubscribe().await?;
        }
        let ctx = self.context();
        self.driver.lock().await.disconnect(&ctx).await?;
        self.transport.close().await
    }

    pub async fn run(mut self) -> Result<()> {
        self.start().await?;
        futures::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    }

    pub fn context(&self) -> DeviceContext {
        DeviceContext::new(
            self.tenant.clone(),
            self.device_id.clone(),
            self.transport.clone(),
        )
    }

    fn registration_params_for(device_id: &str, ttl: u64, driver: &D) -> RegistrationParams {
        let definition = driver.definition();
        let mut identity = driver.identity();
        if identity.device_type.is_none() {
            identity.device_type = Some(definition.device_type.clone());
        }
        RegistrationParams {
            device_id: device_id.to_string(),
            device_ttl: ttl,
            capabilities: definition.capabilities,
            identity,
            status: driver.status(),
            attestation: None,
        }
    }

    async fn subscribe_commands(&mut self) -> Result<()> {
        let subject = subjects::command(&self.tenant, &self.device_id);
        let driver = self.driver.clone();
        let ctx = self.context();
        let transport = self.transport.clone();
        let device_id = self.device_id.clone();
        let ttl = self.ttl;

        let handler: MessageHandler = Arc::new(move |data, reply_subject| {
            let driver = driver.clone();
            let ctx = ctx.clone();
            let transport = transport.clone();
            let device_id = device_id.clone();
            async move {
                let Some(reply_subject) = reply_subject else {
                    return;
                };
                let response = match serde_json::from_slice::<JsonRpcRequest>(&data) {
                    Ok(request) if request.method == "requestRegistration" => {
                        let guard = driver.lock().await;
                        let params =
                            DeviceRuntime::registration_params_for(&device_id, ttl, &*guard);
                        JsonRpcResponse::success(
                            request.id,
                            serde_json::to_value(params).unwrap_or_else(|_| json!({})),
                        )
                    }
                    Ok(request) => {
                        let function_exists = {
                            let guard = driver.lock().await;
                            guard
                                .definition()
                                .capabilities
                                .functions
                                .iter()
                                .any(|f| f.name == request.method)
                        };
                        if !function_exists {
                            JsonRpcResponse::error(
                                Some(request.id),
                                ERR_METHOD_NOT_FOUND,
                                format!("Unknown method: {}", request.method),
                            )
                        } else {
                            let mut guard = driver.lock().await;
                            match guard
                                .handle_rpc(&request.method, request.params, &ctx)
                                .await
                            {
                                Ok(result) => JsonRpcResponse::success(request.id, result),
                                Err(e) => JsonRpcResponse::error(
                                    Some(request.id),
                                    ERR_DRIVER,
                                    e.to_string(),
                                ),
                            }
                        }
                    }
                    Err(e) => {
                        JsonRpcResponse::error(None, crate::protocol::ERR_PARSE, e.to_string())
                    }
                };
                if let Ok(data) = serde_json::to_vec(&response) {
                    let _ = transport.publish(&reply_subject, data).await;
                }
            }
            .boxed()
        });

        let sub = self.transport.subscribe(&subject, handler, false).await?;
        self.subscriptions.push(sub);
        info!("subscribed to commands on {}", subject);
        Ok(())
    }

    async fn subscribe_broadcasts(&mut self) -> Result<()> {
        let subject = subjects::broadcast(&self.tenant);
        let driver = self.driver.clone();
        let ctx = self.context();
        let transport = self.transport.clone();
        let tenant = self.tenant.clone();
        let device_id = self.device_id.clone();

        let handler: MessageHandler = Arc::new(move |data, _reply_subject| {
            let driver = driver.clone();
            let ctx = ctx.clone();
            let transport = transport.clone();
            let tenant = tenant.clone();
            let device_id = device_id.clone();
            async move {
                let Ok(envelope) = serde_json::from_slice::<BroadcastEnvelope>(&data) else {
                    return;
                };
                if !envelope.targets.is_empty() && !envelope.targets.iter().any(|target| target == &device_id) {
                    return;
                }
                if envelope.where_clause.is_some() {
                    warn!("where predicates are not implemented in device-connect-edge-rs v1; skipping broadcast {}", envelope.correlation_id);
                    return;
                }
                if let Some(fire_at) = envelope.fire_at {
                    let delay = fire_at - unix_time_f64();
                    if delay < 0.0 && envelope.on_late == "skip" {
                        return;
                    }
                    if delay > 0.0 {
                        time::sleep(Duration::from_secs_f64(delay)).await;
                    }
                }

                let actually_fired_at = unix_time_f64();
                let mut guard = driver.lock().await;
                let result = guard.handle_rpc(&envelope.function, envelope.params, &ctx).await;
                let reply = match result {
                    Ok(result) => BroadcastReply {
                        correlation_id: envelope.correlation_id.clone(),
                        device_id: device_id.clone(),
                        success: true,
                        result: Some(result),
                        error: None,
                        actually_fired_at,
                    },
                    Err(e) => BroadcastReply {
                        correlation_id: envelope.correlation_id.clone(),
                        device_id: device_id.clone(),
                        success: false,
                        result: None,
                        error: Some(json!({"code": "invoke_failed", "message": e.to_string()})),
                        actually_fired_at,
                    },
                };
                if let Ok(data) = serde_json::to_vec(&reply) {
                    let _ = transport
                        .publish(&subjects::async_reply(&tenant, &device_id, &envelope.correlation_id), data)
                        .await;
                }
            }
            .boxed()
        });

        let sub = self.transport.subscribe(&subject, handler, true).await?;
        self.subscriptions.push(sub);
        info!("subscribed to broadcasts on {}", subject);
        Ok(())
    }

    async fn start_presence(&mut self) -> Result<()> {
        let guard = self.driver.lock().await;
        let definition = guard.definition();
        let announcer = Arc::new(PresenceAnnouncer::new(
            self.transport.clone(),
            self.device_id.clone(),
            self.tenant.clone(),
            definition.capabilities,
            guard.identity(),
            guard.status(),
        ));
        drop(guard);

        let collector = Arc::new(PresenceCollector::new(
            self.transport.clone(),
            self.tenant.clone(),
            Some(self.device_id.clone()),
        ));
        collector.start().await?;
        announcer.start().await?;
        self.presence_announcer = Some(announcer);
        self.presence_collector = Some(collector);
        Ok(())
    }

    fn start_heartbeat(&mut self) {
        let transport = self.transport.clone();
        let subject = subjects::heartbeat(&self.tenant, &self.device_id);
        let device_id = self.device_id.clone();
        let interval = self.heartbeat_interval;
        self.tasks.push(tokio::spawn(async move {
            let mut ticker = time::interval(interval);
            loop {
                ticker.tick().await;
                let beat = Heartbeat {
                    device_id: device_id.clone(),
                    ts: unix_time_f64(),
                    extra: Default::default(),
                };
                match serde_json::to_vec(&beat) {
                    Ok(data) => {
                        if let Err(e) = transport.publish(&subject, data).await {
                            debug!("heartbeat publish failed: {}", e);
                        }
                    }
                    Err(e) => debug!("heartbeat serialization failed: {}", e),
                }
            }
        }));
    }

    pub async fn register_once(&self) -> Result<()> {
        let guard = self.driver.lock().await;
        let params = Self::registration_params_for(&self.device_id, self.ttl, &*guard);
        drop(guard);
        let request = build_registration_request(params);
        self.transport
            .request(
                &subjects::registry(&self.tenant),
                serde_json::to_vec(&request)?,
                Duration::from_secs(15),
            )
            .await
            .map(|_| ())
    }

    pub async fn emit(&self, event_name: &str, payload: Value) -> Result<()> {
        validate_event_name(event_name)?;
        let note = EventNotification::new(
            event_name,
            crate::protocol::event_payload_with_defaults(payload),
        );
        self.transport
            .publish(
                &subjects::event(&self.tenant, &self.device_id, event_name),
                serde_json::to_vec(&note)?,
            )
            .await
    }
}

pub struct DeviceRuntimeBuilder<D: DeviceDriver> {
    driver: D,
    device_id: Option<String>,
    tenant: String,
    ttl: u64,
    heartbeat_interval: Option<Duration>,
    allow_insecure: bool,
    transport: Option<TransportRef>,
}

impl<D: DeviceDriver> DeviceRuntimeBuilder<D> {
    fn new(driver: D) -> Self {
        Self {
            driver,
            device_id: None,
            tenant: "default".to_string(),
            ttl: 15,
            heartbeat_interval: None,
            allow_insecure: false,
            transport: None,
        }
    }

    pub fn device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = Some(device_id.into());
        self
    }

    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = tenant.into();
        self
    }

    pub fn ttl(mut self, ttl: u64) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = Some(interval);
        self
    }

    pub fn allow_insecure(mut self, allow_insecure: bool) -> Self {
        self.allow_insecure = allow_insecure;
        self
    }

    pub fn transport(mut self, transport: TransportRef) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn build(self) -> Result<DeviceRuntime<D>> {
        let device_id = self.device_id.unwrap_or_else(|| {
            format!(
                "device-{}",
                uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
            )
        });
        validate_device_id(&device_id)?;
        let transport = self
            .transport
            .unwrap_or_else(|| Arc::new(ZenohTransport::from_env()));
        Ok(DeviceRuntime {
            driver: Arc::new(Mutex::new(self.driver)),
            device_id,
            tenant: self.tenant,
            ttl: self.ttl,
            heartbeat_interval: self
                .heartbeat_interval
                .unwrap_or_else(|| Duration::from_secs(std::cmp::max(1, self.ttl / 3))),
            allow_insecure: self.allow_insecure,
            transport,
            subscriptions: Vec::new(),
            tasks: Vec::new(),
            presence_announcer: None,
            presence_collector: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::DeviceDefinition;
    use crate::protocol::ERR_METHOD_NOT_FOUND;
    use crate::transport::{NoopSubscription, Subscription, Transport};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::BTreeMap;
    use tokio::sync::RwLock;

    #[derive(Default)]
    struct MockTransport {
        published: RwLock<Vec<(String, Vec<u8>)>>,
        handlers: RwLock<BTreeMap<String, MessageHandler>>,
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn connect(&self) -> Result<()> {
            Ok(())
        }

        async fn publish(&self, subject: &str, data: Vec<u8>) -> Result<()> {
            self.published
                .write()
                .await
                .push((subject.to_string(), data));
            Ok(())
        }

        async fn subscribe(
            &self,
            subject: &str,
            handler: MessageHandler,
            _subscribe_only: bool,
        ) -> Result<Box<dyn Subscription>> {
            self.handlers
                .write()
                .await
                .insert(subject.to_string(), handler);
            Ok(Box::new(NoopSubscription))
        }

        async fn request(
            &self,
            _subject: &str,
            _data: Vec<u8>,
            _timeout: Duration,
        ) -> Result<Vec<u8>> {
            Err(DeviceConnectError::RequestTimeout)
        }

        async fn close(&self) -> Result<()> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }
    }

    struct Sensor;

    #[async_trait]
    impl DeviceDriver for Sensor {
        fn definition(&self) -> DeviceDefinition {
            DeviceDefinition::builder("sensor")
                .function(crate::types::FunctionDef::new("get_reading"))
                .build()
        }

        async fn handle_rpc(
            &mut self,
            function: &str,
            _params: Value,
            _ctx: &DeviceContext,
        ) -> Result<Value> {
            match function {
                "get_reading" => Ok(json!({"temperature": 22.5, "humidity": 45})),
                other => Err(DeviceConnectError::Driver(format!("unknown {other}"))),
            }
        }
    }

    #[tokio::test]
    async fn unknown_methods_return_python_error_code() {
        let transport = Arc::new(MockTransport::default());
        let mut runtime = DeviceRuntime::builder(Sensor)
            .device_id("sensor-001")
            .allow_insecure(true)
            .transport(transport.clone())
            .build()
            .unwrap();
        runtime.start().await.unwrap();
        let handler = transport
            .handlers
            .read()
            .await
            .get("device-connect.default.sensor-001.cmd")
            .unwrap()
            .clone();
        handler(
            serde_json::to_vec(
                &json!({"jsonrpc": "2.0", "id": "1", "method": "nope", "params": {}}),
            )
            .unwrap(),
            Some("_reply".to_string()),
        )
        .await;
        let published = transport.published.read().await;
        let (_, data) = published.iter().find(|(s, _)| s == "_reply").unwrap();
        let value: Value = serde_json::from_slice(data).unwrap();
        assert_eq!(value["error"]["code"], ERR_METHOD_NOT_FOUND);
        runtime.stop().await.unwrap();
    }

    #[tokio::test]
    async fn emits_python_event_notification_shape() {
        let transport = Arc::new(MockTransport::default());
        let runtime = DeviceRuntime::builder(Sensor)
            .device_id("sensor-001")
            .allow_insecure(true)
            .transport(transport.clone())
            .build()
            .unwrap();
        runtime
            .emit("reading", json!({"temperature": 22.5}))
            .await
            .unwrap();
        let published = transport.published.read().await;
        let (subject, data) = published.last().unwrap();
        assert_eq!(subject, "device-connect.default.sensor-001.event.reading");
        let value: Value = serde_json::from_slice(data).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["method"], "reading");
        assert!(value["params"]["event_id"].is_string());
    }
}
