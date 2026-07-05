# device-connect-edge-rs Feature-Parity Plan

## Summary

Build `device-connect-edge-rs` as an idiomatic Rust edge SDK with Zenoh-first transport parity, a trait/builder driver API, and a protocol core shaped for future Python reuse without adding PyO3 bindings yet.

The first implementation should prioritize wire compatibility with Python devices and agents: same subjects, JSON-RPC envelopes, presence payloads, registration payloads, heartbeat behavior, event subjects, and broadcast reply shape.

Current paused state:

- The root Rust workspace and `packages/device-connect-edge-rs` scaffold have been expanded.
- Core modules now exist for `types`, `protocol`, `transport`, `driver`, `discovery`, `runtime`, `transports::zenoh`, and `errors`.
- `cargo fmt` has run.
- `cargo test` was interrupted because one async runtime test was still active after the other unit tests passed; investigate cleanup/hanging behavior before continuing implementation.

## Key Changes

- Fix the Rust workspace scaffold:
  - Move root metadata into `[workspace.package]`.
  - Set `resolver = "3"` for edition 2024.
  - Keep `packages/device-connect-edge-rs` as the first Rust crate.

- Organize the crate into stable modules:
  - `types`: capabilities, identity, status, functions, events, labels, validation.
  - `protocol`: subject builders, JSON-RPC types, registration, heartbeat, presence, broadcast envelopes.
  - `transport`: async `Transport` trait.
  - `transports::zenoh`: first concrete transport.
  - `runtime`: lifecycle, command handling, D2D presence, event queue, heartbeat, shutdown.
  - `driver`: `DeviceDriver` trait plus builder helpers.
  - `discovery`: Python-compatible presence announcer/collector.
  - `errors`: typed SDK errors.

- Use idiomatic Rust dependencies:
  - `tokio`, `serde`, `serde_json`, `schemars`, `zenoh`, `async-trait`, `tracing`, `thiserror`, `uuid`, `time`.

- Public API shape:
  - `DeviceRuntime::builder(driver)` configures `device_id`, `tenant`, `ttl`, transport, credentials, and Zenoh endpoints.
  - `DeviceDriver` exposes `device_type`, `identity`, `status`, `capabilities`, `connect`, `disconnect`, and `handle_rpc`.
  - `DeviceContext` gives drivers `emit`, `invoke_remote`, `list_devices`, `get_device`, and raw transport access.
  - Driver metadata uses explicit builders, not proc macros in v1.

- Feature parity target:
  - Implement v1: Zenoh D2D mode, direct unicast `ZENOH_CONNECT`, multicast interface option, presence discovery, JSON-RPC command handling, remote invoke, events, registration payloads, heartbeat publishing, graceful departure, broadcast handling, `fire_at`, and target self-election.
  - Defer: NATS, MQTT, commissioning server, full OpenTelemetry export, CEL `where` predicates, Python bindings.

## Test Plan

- Rust unit tests:
  - Type serialization matches Python payload shapes.
  - Subject builders match `device-connect.{tenant}.{device_id}.*`.
  - JSON-RPC success/error envelopes match Python.
  - Event name and device ID validation match Python behavior.
  - Unknown methods return `-32601`; handler failures return `-32000`.

- Cross-language integration tests:
  - Python agent discovers Rust device over Zenoh D2D.
  - Python agent invokes Rust device RPC.
  - Rust device invokes Python device RPC.
  - Rust and Python devices exchange events.
  - Rust device handles broadcast envelopes and emits Python-compatible async replies.

- CI:
  - Add Rust jobs for `cargo check`, `cargo test`, and later `cargo clippy`.
  - Keep Python CI unchanged.
  - Make cross-language tests opt-in initially if they require Docker or live Zenoh setup.

## Assumptions

- First transport is Zenoh only.
- First Rust API is trait/builder based, not macro based.
- No PyO3/maturin bindings in the first implementation.
- Feature parity means wire/runtime compatibility, not Python decorator parity.
- NATS and MQTT are added after Zenoh runtime and cross-language conformance pass.
