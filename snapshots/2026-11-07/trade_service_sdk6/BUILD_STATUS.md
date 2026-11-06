# SDK 6 isolated safety candidate

Checked: 2026-11-06. Source copied from the frozen `releases/sdk-6.0.2-legacy` release into this directory. Production `.env`, wallet passphrase, SQLite and `data` were not copied. `current` and the 18788 tunnel were not changed.

The isolated copy changes `src/sdk_client.rs` and `src/executor.rs` only:

- one live SDK order attempts one lane, so an uncertain send is not rebuilt on a fallback lane;
- an invisible signature or transport error remains `unknown` for reconciliation;
- a locally signed simulation fingerprint is not exposed as a submitted signature.

`cargo fmt --all -- --check` passed. The initial `cargo test --locked --offline` could not resolve the pinned `protocol-zap` Git dependency, and the original `https://github.com/MeteoraAg/zap-program` URL returned HTTP 404. On 2026-11-07, the exact pinned commit `064c58b317b9a85f212c0de72caea286fc72fdb4` was obtained from the public `2mm-ai/meteora-zap-program` fork. Its tree is `40424609ca5e5f6ec8169fca2f792282c491cc99`; the complete commit was imported into the local Cargo cache under the original URL, without changing `Cargo.lock`. `cargo fetch --locked` then succeeded. The first source check found that this isolated copy lacked the migration SQL included by `src/persistence.rs`; the file was copied byte for byte from the frozen SDK 6 release (SHA-256 `7e4abf94cc9282f7a2e2145123e5783200b750b35a240b6b42fc27de9d89258a`). With the pinned Rust 1.99 toolchain, `cargo check --locked --offline`, `cargo test --locked --offline` (50 passed) and `cargo build --release --locked --offline` passed. The isolated binary SHA-256 is `d7adf52bb04e2b9b1ba35e5edba5c89c7f950d8b06566695600b6ace397d8311`, archived at `/root/solana/trade_service/builds/sdk6-send-safe-d7adf52bb04e`. This candidate has **not passed live reconciliation** and must not be deployed.

The reviewable `legacy_send_safety.patch` contains only the two changed Rust files and passes `patch --dry-run -p1` against the frozen source. Its four focused status tests passed with the full locked suite.

The binary also started on a random loopback port with a one-time wallet, temporary durable SQLite and an intentionally unreachable RPC. `/health` passed; `/ready` correctly reported `ready=false`, `rpc_ok=false`, and one wallet. This verifies startup and database initialization, not a funded trade simulation. The process was stopped after the check.

An isolated HTTP test used a copy of the staging wallet database, one funded wallet, temporary durable SQLite, a random loopback port, and explicit `simulate=true` requests. A PumpSwap single-pool buy and a Raydium CPMM single-pool sell both reached `simulated` with no signature. Both idempotency keys returned their original orders, and restart readback preserved both. The disposable database contains two trades and zero signed rows; the copied wallet database was removed after the run. The result is in `/root/solana/trade_service/evidence/sdk6-sdk11-priority-20261107/SDK6_SINGLE_POOL_RESULT.json`. This verifies two simulations, not any live send.

A separate copy of the two-order database was then seeded with one `submitting` row to model a process interruption before a signature is recorded. On startup the service recovered it as `unknown`; an identical idempotency request returned the original order, and the row count remained two. This verifies durable HTTP recovery without replay. It does not inject a real network timeout after a transaction was sent.

Next: fault-inject an actual ambiguous send and verify signature or order reconciliation before considering deployment. SDK 11 candidate development continues independently.
