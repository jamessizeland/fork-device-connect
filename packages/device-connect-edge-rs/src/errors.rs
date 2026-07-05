// Copyright (c) 2024-2026, Arm Limited and Contributors. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DeviceConnectError>;

#[derive(Debug, Error)]
pub enum DeviceConnectError {
    #[error("invalid device id {0:?}; must match ^[a-zA-Z0-9][a-zA-Z0-9._-]{{0,254}}$")]
    InvalidDeviceId(String),

    #[error("invalid event name {0:?}; must match ^[A-Za-z0-9_-]+$")]
    InvalidEventName(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("request timed out")]
    RequestTimeout,

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("driver error: {0}")]
    Driver(String),

    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
}
