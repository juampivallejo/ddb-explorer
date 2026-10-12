# gpui-demo

A DynamoDB explorer in progress, built with Rust and
[GPUI Kit](https://gpui-kit.com/) for Linux with Wayland.
The app loads tables from local DynamoDB or your terminal's AWS session.
Select a table to query its primary key or a GSI, or explicitly confirm a scan.
Reads return one page at a time. Select an item to view and copy plain JSON.
Sets become arrays and binary values use base64; numbers retain their precision.

## Build and run

Requires a Wayland session, Vulkan-capable graphics, and installed fonts.
The Nix shell provides Rust and the native build dependencies:

```sh
nix-shell
cargo build
./target/debug/gpui-demo
```

For a release binary, use `cargo build --release` and run
`./target/release/gpui-demo` from your terminal.
Without Nix, install the dependencies listed in `shell.nix` and a Rust
toolchain supporting edition 2024. On NixOS, enable `hardware.graphics.enable`.

## Local DynamoDB

Requires Docker with Compose and a running daemon. From the development shell:

```sh
docker compose up -d
./scripts/seed
./scripts/ddb list-tables
```

DynamoDB runs at `http://127.0.0.1:8000` with persistent local data.
Stop it with `docker compose down`.
See [scripts/README.md](scripts/README.md) for fixtures, queries, and reset commands.

## Development

```sh
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
```

`src/main.rs` starts the app, `src/app.rs` holds the explorer view and state,
`src/ui/` contains shared components, and `src/dynamodb/` handles AWS access.
