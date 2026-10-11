# gpui-demo

A DynamoDB explorer in progress, built with Rust and
[GPUI](https://docs.rs/gpui/0.2.2/gpui/) for Linux with Wayland.
The app currently has connection and read-mode selectors, a table sidebar, and
an empty item workspace. DynamoDB access is not wired up yet.

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
`src/ui/` contains shared components, and `src/dynamodb/` defines connection
and read-operation choices.
