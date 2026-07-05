// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

//! Rust edge SDK for Device Connect.
//!
//! This crate keeps the Rust API idiomatic while preserving the Python SDK's
//! wire protocol: subjects, JSON-RPC envelopes, presence announcements,
//! registration payloads, heartbeats, events, and broadcast replies.

pub mod discovery;
pub mod driver;
pub mod errors;
pub mod protocol;
pub mod runtime;
pub mod transport;
pub mod transports;
pub mod types;

pub use discovery::{PresenceAnnouncer, PresenceCollector};
pub use driver::{DeviceContext, DeviceDefinition, DeviceDefinitionBuilder, DeviceDriver};
pub use errors::{DeviceConnectError, Result};
pub use runtime::{DeviceRuntime, DeviceRuntimeBuilder};
pub use transport::{MessageHandler, Subscription, Transport, TransportRef};
pub use types::{
    DeviceCapabilities, DeviceIdentity, DeviceStatus, EventDef, FunctionDef, LabelValue,
};
