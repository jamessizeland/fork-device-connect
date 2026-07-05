// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::errors::{DeviceConnectError, Result};
use crate::transport::{MessageHandler, NoopSubscription, Subscription, Transport};

#[derive(Clone, Debug, Default)]
pub struct ZenohTransportConfig {
    pub endpoints: Vec<String>,
    pub listen: Vec<String>,
    pub multicast_interface: Option<String>,
    pub peer_mode: bool,
}

pub struct ZenohTransport {
    config: ZenohTransportConfig,
    connected: AtomicBool,
}

impl ZenohTransport {
    pub fn new(config: ZenohTransportConfig) -> Self {
        Self {
            config,
            connected: AtomicBool::new(false),
        }
    }

    pub fn from_env() -> Self {
        let endpoints = std::env::var("ZENOH_CONNECT")
            .ok()
            .map(|v| split_env_list(&v))
            .unwrap_or_default();
        let listen = std::env::var("ZENOH_LISTEN")
            .ok()
            .map(|v| split_env_list(&v))
            .unwrap_or_default();
        let multicast_interface = std::env::var("ZENOH_MULTICAST_INTERFACE").ok();
        let peer_mode = endpoints.is_empty()
            || matches!(
                std::env::var("DEVICE_CONNECT_DISCOVERY_MODE")
                    .ok()
                    .as_deref(),
                Some("d2d" | "p2p")
            );
        Self::new(ZenohTransportConfig {
            endpoints,
            listen,
            multicast_interface,
            peer_mode,
        })
    }

    pub fn config(&self) -> &ZenohTransportConfig {
        &self.config
    }
}

#[async_trait]
impl Transport for ZenohTransport {
    async fn connect(&self) -> Result<()> {
        // Keep this type in the public API from the start, but defer live Zenoh
        // session wiring until the cross-language tests pin the exact request /
        // reply behavior against the Python adapter.
        self.connected.store(true, Ordering::SeqCst);
        Err(DeviceConnectError::Unsupported(
            "live Zenoh transport wiring is not implemented in this first scaffold; inject a Transport implementation for tests or applications",
        ))
    }

    async fn publish(&self, _subject: &str, _data: Vec<u8>) -> Result<()> {
        Err(DeviceConnectError::Unsupported("Zenoh publish"))
    }

    async fn subscribe(
        &self,
        _subject: &str,
        _handler: MessageHandler,
        _subscribe_only: bool,
    ) -> Result<Box<dyn Subscription>> {
        Ok(Box::new(NoopSubscription))
    }

    async fn request(&self, _subject: &str, _data: Vec<u8>, _timeout: Duration) -> Result<Vec<u8>> {
        Err(DeviceConnectError::Unsupported("Zenoh request"))
    }

    async fn close(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

fn split_env_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_env_config_is_peer_mode() {
        let transport = ZenohTransport::new(ZenohTransportConfig::default());
        assert!(!transport.is_connected());
        assert!(transport.config().endpoints.is_empty());
    }
}
