# Rust Device Connect API Proposal

This proposal sketches a Rust-first API for Device Connect while preserving a path for the existing Python `DeviceDriver`, `@rpc`, and `@emit` surface through a PyO3 compatibility layer.

The main design goal is to make the Rust core the stable runtime and protocol implementation, with Python becoming one frontend over that core rather than a parallel implementation.

## Goals

- Provide an idiomatic Rust API for defining devices, RPCs, events, capabilities, and clients.
- Keep the Python surface broadly intact through PyO3, especially `DeviceDriver` and `@rpc`.
- Support typed request and response models in Rust.
- Generate capability metadata and JSON schemas from Rust types.
- Allow a fast static dispatch path for pure Rust devices.
- Keep a dynamic dispatch path for Python compatibility, plugins, and runtime registration.
- Hide transport and wire protocol details from normal device authors.

## Current Python Shape

In the Python SDK, `@rpc` is a decorator that performs two jobs:

1. It wraps the method with runtime behavior: tracing, logging, metrics, caller identity, and error reporting.
2. It marks the wrapped method with metadata so `DeviceDriver` can discover it later.

Conceptually:

```python
@rpc()
async def ping(self) -> dict:
    return {"ok": True}
```

The decorator attaches attributes similar to:

```python
wrapper._is_device_function = True
wrapper._function_name = "ping"
wrapper._description = "..."
wrapper._arg_descriptions = {...}
wrapper._labels = {...}
wrapper._original_func = func
```

`DeviceDriver` then scans the instance, finds methods marked with `_is_device_function`, builds `FunctionDef` values from their metadata and type hints, and dispatches incoming calls by function name.

That dynamic attribute model is natural in Python. In Rust, the equivalent should be explicit registration and typed dispatch.

## Rust Core API

The Rust core should model an RPC as a first-class value with an associated request and response type.

```rust
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};

#[async_trait]
pub trait Rpc: Send + Sync + 'static {
    const NAME: &'static str;

    type Request: DeserializeOwned + JsonSchema + Send + 'static;
    type Response: Serialize + JsonSchema + Send + 'static;

    fn description(&self) -> &'static str {
        ""
    }

    fn labels(&self) -> Labels {
        Labels::default()
    }

    async fn call(&self, ctx: RpcContext, request: Self::Request) -> Result<Self::Response>;
}
```

Example RPC with no input:

```rust
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct PingResponse {
    pub ok: bool,
}

pub struct Ping;

#[async_trait::async_trait]
impl Rpc for Ping {
    const NAME: &'static str = "ping";

    type Request = ();
    type Response = PingResponse;

    fn description(&self) -> &'static str {
        "Return liveness status"
    }

    async fn call(&self, _ctx: RpcContext, _request: ()) -> Result<PingResponse> {
        Ok(PingResponse { ok: true })
    }
}
```

Example RPC with typed input and output:

```rust
#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct CaptureImageRequest {
    pub resolution: String,
    pub format: String,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct CaptureImageResponse {
    pub image_b64: String,
}

pub struct CaptureImage;

#[async_trait::async_trait]
impl Rpc for CaptureImage {
    const NAME: &'static str = "capture_image";

    type Request = CaptureImageRequest;
    type Response = CaptureImageResponse;

    fn description(&self) -> &'static str {
        "Capture an image from the camera"
    }

    async fn call(
        &self,
        _ctx: RpcContext,
        request: CaptureImageRequest,
    ) -> Result<CaptureImageResponse> {
        let _resolution = request.resolution;
        let _format = request.format;

        Ok(CaptureImageResponse {
            image_b64: "...".to_string(),
        })
    }
}
```

This maps directly to the job performed by Python `@rpc`, but in a Rust-native way:

- `Rpc::NAME` replaces `_function_name`.
- `description()` replaces `_description`.
- `labels()` replaces `_labels`.
- `Request: JsonSchema` replaces Python signature inspection for parameters.
- `Response: JsonSchema` adds output schema support.
- `call()` replaces invoking the decorated Python method.

## Runtime Context

Python currently exposes caller identity through a context variable. In Rust, this should be explicit:

```rust
#[derive(Clone, Debug)]
pub struct RpcContext {
    pub device_id: String,
    pub source_device: Option<String>,
    pub trace_id: Option<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}
```

RPC handlers receive `RpcContext` as an argument instead of relying on ambient global state.

## Dynamic Dispatch Path

The runtime still needs a dynamic boundary when calls arrive from the network by string name. Different RPCs have different request and response types, so they cannot all be stored directly in one homogeneous collection without erasing their concrete types.

For that case, use an erased trait:

```rust
#[async_trait::async_trait]
pub trait ErasedRpc: Send + Sync {
    fn def(&self) -> FunctionDef;

    async fn call_json(
        &self,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value>;
}
```

Every typed RPC can automatically implement this erased interface:

```rust
#[async_trait::async_trait]
impl<T> ErasedRpc for T
where
    T: Rpc,
{
    fn def(&self) -> FunctionDef {
        FunctionDef {
            name: T::NAME.to_string(),
            description: self.description().to_string(),
            parameters: schema_for::<T::Request>(),
            response: Some(schema_for::<T::Response>()),
            labels: Some(self.labels()),
            tags: Vec::new(),
        }
    }

    async fn call_json(
        &self,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let typed_request: T::Request = serde_json::from_value(request)?;
        let typed_response = self.call(ctx, typed_request).await?;
        Ok(serde_json::to_value(typed_response)?)
    }
}
```

This enables a flexible registry:

```rust
pub struct DynamicRpcRegistry {
    rpcs: std::collections::HashMap<String, Box<dyn ErasedRpc>>,
}

impl DynamicRpcRegistry {
    pub fn register<R>(&mut self, rpc: R)
    where
        R: Rpc,
    {
        self.rpcs.insert(R::NAME.to_string(), Box::new(rpc));
    }

    pub async fn dispatch(
        &self,
        name: &str,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let rpc = self
            .rpcs
            .get(name)
            .ok_or_else(|| Error::unknown_rpc(name))?;

        rpc.call_json(ctx, request).await
    }
}
```

This path is useful for:

- Python compatibility.
- Runtime-loaded capabilities.
- Plugin systems.
- Tests and mock devices.
- Any device where the RPC set is not known at compile time.

## Static Dispatch Path

For pure Rust devices with a statically known RPC set, the runtime can avoid a dynamic registry and use generated or hand-written match dispatch.

The common trait could be:

```rust
#[async_trait::async_trait]
pub trait DeviceRpcSet: Send + Sync {
    fn capabilities(&self) -> DeviceCapabilities;

    async fn dispatch(
        &self,
        name: &str,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value>;
}
```

Example hand-written implementation:

```rust
pub struct CameraDevice {
    ping: Ping,
    capture_image: CaptureImage,
}

#[async_trait::async_trait]
impl DeviceRpcSet for CameraDevice {
    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities::new()
            .rpc(self.ping.def())
            .rpc(self.capture_image.def())
    }

    async fn dispatch(
        &self,
        name: &str,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match name {
            Ping::NAME => self.ping.call_json(ctx, request).await,
            CaptureImage::NAME => self.capture_image.call_json(ctx, request).await,
            _ => Err(Error::unknown_rpc(name)),
        }
    }
}
```

This still converts JSON at the transport boundary, but it avoids:

- Registry lookup through boxed trait objects.
- Per-RPC heap allocation in the dispatch table.
- Dynamic handler registration.

The performance benefit is likely smaller than the cost of transport, JSON conversion, async scheduling, and Python interop. Still, static dispatch is a good design option for pure Rust users and embedded-style deployments.

## Macro Ergonomics

The static implementation can later be generated by a macro:

```rust
#[derive(DeviceRpcSet)]
pub struct CameraDevice {
    #[rpc]
    ping: Ping,

    #[rpc]
    capture_image: CaptureImage,
}
```

Or with a declarative macro:

```rust
device! {
    struct CameraDevice {
        identity: camera_identity(),
        rpcs: [
            Ping,
            CaptureImage,
        ],
        events: [
            MotionDetected,
        ],
    }
}
```

The core should not depend on macros. Macros should be optional sugar over the same `Rpc`, `ErasedRpc`, and `DeviceRpcSet` traits.

## Device Builder API

The user-facing Rust SDK should make simple devices concise:

```rust
#[tokio::main]
async fn main() -> Result<()> {
    Device::builder("camera-001")
        .identity(
            DeviceIdentity::new()
                .device_type("camera")
                .manufacturer("Acme")
                .model("Cam-1000"),
        )
        .rpc(Ping)
        .rpc(CaptureImage)
        .connect_zenoh()
        .await?
        .serve()
        .await
}
```

For closure-based handlers, a builder can provide a lighter path:

```rust
Device::builder("device-a")
    .identity(DeviceIdentity::new().device_type("demo_device"))
    .rpc("ping")
        .description("Return liveness status")
        .handler(|_ctx: RpcContext, _: ()| async move {
            Ok(serde_json::json!({ "ok": true }))
        })
    .connect_zenoh()
    .await?
    .serve()
    .await?;
```

The trait-based form is better for reusable libraries and static dispatch. The closure-based form is better for small examples and application-local devices.

## Client API

The Rust client should also hide transport details:

```rust
let client = EdgeClient::connect_zenoh().await?;

let devices = client
    .discover()
    .label("category", "camera")
    .online_only()
    .await?;

let image: CaptureImageResponse = client
    .device("camera-001")
    .call("capture_image", CaptureImageRequest {
        resolution: "1080p".to_string(),
        format: "jpeg".to_string(),
    })
    .await?;

let mut events = client
    .device("camera-001")
    .subscribe::<MotionDetected>("motion_detected")
    .await?;
```

## PyO3 Compatibility Layer

The Python API can remain decorator-based while delegating runtime behavior to Rust.

Current Python surface:

```python
class DeviceADriver(DeviceDriver):
    device_type = "demo_device"

    @rpc()
    async def ping(self) -> dict:
        return {"ok": True}
```

Compatibility implementation:

```text
@rpc
  -> inspect Python signature and docstring
  -> preserve Python metadata attributes for compatibility
  -> create a PythonRpcHandler
  -> register that handler with the Rust DynamicRpcRegistry
```

Rust-side handler shape:

```rust
pub struct PythonRpcHandler {
    name: String,
    def: FunctionDef,
    callable: pyo3::Py<pyo3::PyAny>,
}

#[async_trait::async_trait]
impl ErasedRpc for PythonRpcHandler {
    fn def(&self) -> FunctionDef {
        self.def.clone()
    }

    async fn call_json(
        &self,
        ctx: RpcContext,
        request: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Acquire the GIL.
        // Convert RpcContext and request JSON to Python values.
        // Call the Python function.
        // Await the coroutine if needed.
        // Convert the Python result back to serde_json::Value.
        todo!()
    }
}
```

This keeps Python compatibility dynamic while making the Rust core authoritative.

## Proposed Crate Boundaries

One possible package layout:

```text
device-connect-core
  Protocol types, capabilities, schemas, RPC traits, dispatch traits, errors.

device-connect-edge
  Rust SDK ergonomics: DeviceBuilder, EdgeClient, Zenoh transport integration.

device-connect-python
  PyO3 compatibility layer preserving DeviceDriver, @rpc, @emit, and current Python behavior.
```

The current repository may not keep this exact structure during the rework. The important design boundary is that the protocol/runtime core should not depend on Python, and the Python layer should adapt to the Rust registry rather than reimplementing it.

## Recommended Direction

Build the Rust core around these primitives:

- `Rpc` for typed Rust request/response handlers.
- `ErasedRpc` for dynamic runtime dispatch.
- `DeviceRpcSet` for static match-based dispatch.
- `DynamicRpcRegistry` for Python compatibility and plugins.
- `RpcContext` for source-device identity, trace metadata, and per-call context.
- `DeviceBuilder` and `EdgeClient` as the high-level ergonomic API.

This design keeps the Python decorator model available, but does not force Rust users into a Python-shaped API. Pure Rust devices can be typed, static, and macro-friendly. Python-backed devices can remain dynamic and compatible with the existing surface.
