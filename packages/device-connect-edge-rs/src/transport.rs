// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

use crate::errors::Result;

pub type MessageHandler =
    Arc<dyn Fn(Vec<u8>, Option<String>) -> BoxFuture<'static, ()> + Send + Sync>;
pub type TransportRef = Arc<dyn Transport>;

#[async_trait]
pub trait Subscription: Send + Sync {
    async fn unsubscribe(&self) -> Result<()>;
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&self) -> Result<()>;

    async fn publish(&self, subject: &str, data: Vec<u8>) -> Result<()>;

    async fn subscribe(
        &self,
        subject: &str,
        handler: MessageHandler,
        subscribe_only: bool,
    ) -> Result<Box<dyn Subscription>>;

    async fn request(&self, subject: &str, data: Vec<u8>, timeout: Duration) -> Result<Vec<u8>>;

    async fn close(&self) -> Result<()>;

    fn is_connected(&self) -> bool;
}

pub struct NoopSubscription;

#[async_trait]
impl Subscription for NoopSubscription {
    async fn unsubscribe(&self) -> Result<()> {
        Ok(())
    }
}
