// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use futures::FutureExt;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time;

use crate::errors::Result;
use crate::protocol::{Presence, subjects, unix_time_f64};
use crate::transport::{MessageHandler, Subscription, TransportRef};
use crate::types::{DeviceCapabilities, DeviceIdentity, DeviceStatus};

pub struct PresenceAnnouncer {
    transport: TransportRef,
    device_id: String,
    tenant: String,
    capabilities: DeviceCapabilities,
    identity: DeviceIdentity,
    status: DeviceStatus,
    interval: Duration,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl PresenceAnnouncer {
    pub fn new(
        transport: TransportRef,
        device_id: impl Into<String>,
        tenant: impl Into<String>,
        capabilities: DeviceCapabilities,
        identity: DeviceIdentity,
        status: DeviceStatus,
    ) -> Self {
        Self {
            transport,
            device_id: device_id.into(),
            tenant: tenant.into(),
            capabilities,
            identity,
            status,
            interval: Duration::from_secs(5),
            task: Mutex::new(None),
        }
    }

    pub async fn start(&self) -> Result<()> {
        self.publish_once().await?;
        let mut slot = self.task.lock().await;
        if slot.is_some() {
            return Ok(());
        }

        let transport = self.transport.clone();
        let subject = subjects::presence(&self.tenant, &self.device_id);
        let mut ticker = time::interval(self.interval);
        let payload = self.presence();
        *slot = Some(tokio::spawn(async move {
            loop {
                ticker.tick().await;
                let Ok(data) = serde_json::to_vec(&payload) else {
                    continue;
                };
                let _ = transport.publish(&subject, data).await;
            }
        }));
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(task) = self.task.lock().await.take() {
            task.abort();
        }
        self.transport
            .publish(
                &subjects::presence(&self.tenant, &self.device_id),
                serde_json::to_vec(&Presence::departing(&self.device_id, &self.tenant))?,
            )
            .await
    }

    pub async fn publish_once(&self) -> Result<()> {
        self.transport
            .publish(
                &subjects::presence(&self.tenant, &self.device_id),
                serde_json::to_vec(&self.presence())?,
            )
            .await
    }

    fn presence(&self) -> Presence {
        Presence {
            device_id: self.device_id.clone(),
            tenant: self.tenant.clone(),
            capabilities: self.capabilities.clone(),
            identity: self.identity.clone(),
            status: self.status.clone(),
            ts: unix_time_f64(),
            departing: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub presence: Presence,
    pub last_seen: f64,
}

pub struct PresenceCollector {
    transport: TransportRef,
    tenant: String,
    self_device_id: Option<String>,
    peers: Arc<RwLock<BTreeMap<String, Peer>>>,
    subscription: Mutex<Option<Box<dyn Subscription>>>,
}

impl PresenceCollector {
    pub fn new(
        transport: TransportRef,
        tenant: impl Into<String>,
        self_device_id: Option<String>,
    ) -> Self {
        Self {
            transport,
            tenant: tenant.into(),
            self_device_id,
            peers: Arc::new(RwLock::new(BTreeMap::new())),
            subscription: Mutex::new(None),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let peers = self.peers.clone();
        let self_id = self.self_device_id.clone();
        let handler: MessageHandler = Arc::new(move |data, _reply| {
            let peers = peers.clone();
            let self_id = self_id.clone();
            async move {
                let Ok(presence) = serde_json::from_slice::<Presence>(&data) else {
                    return;
                };
                if self_id.as_deref() == Some(presence.device_id.as_str()) {
                    return;
                }
                let mut guard = peers.write().await;
                if presence.departing.unwrap_or(false) {
                    guard.remove(&presence.device_id);
                } else {
                    guard.insert(
                        presence.device_id.clone(),
                        Peer {
                            presence,
                            last_seen: unix_time_f64(),
                        },
                    );
                }
            }
            .boxed()
        });
        let sub = self
            .transport
            .subscribe(&subjects::presence_wildcard(&self.tenant), handler, true)
            .await?;
        *self.subscription.lock().await = Some(sub);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(sub) = self.subscription.lock().await.take() {
            sub.unsubscribe().await?;
        }
        Ok(())
    }

    pub async fn list_devices(&self) -> Vec<Presence> {
        self.peers
            .read()
            .await
            .values()
            .map(|p| p.presence.clone())
            .collect()
    }

    pub async fn get_device(&self, device_id: &str) -> Option<Presence> {
        self.peers
            .read()
            .await
            .get(device_id)
            .map(|p| p.presence.clone())
    }
}
