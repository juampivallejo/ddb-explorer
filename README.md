# ddb-explorer

A DynamoDB explorer in progress, built with Rust and
[GPUI Kit](https://gpui-kit.com/) for Linux with Wayland.
The app loads tables from local DynamoDB or your terminal's AWS session.
Select a table to query its primary key or a GSI, or explicitly confirm a scan.
Reads return one page at a time. Select an item to view and copy plain JSON.
Sets become arrays and binary values use base64; numbers retain their precision.
Filter table names or star favorites to keep them at the top; **Favorites only**
narrows the list further. Local and terminal favorites are saved separately.
History records successful queries and up to 50 returned primary keys per execution.
Use **Save snapshot** to keep an item's historical JSON; **Restore query** fills
the form on your current connection without running it.

## Build and run

Requires a Wayland session, Vulkan-capable graphics, and installed fonts.
The Nix shell provides Rust and the native build dependencies:

```sh
nix-shell
cargo build
./target/debug/ddb-explorer
```

For a release binary, use `cargo build --release` and run
`./target/release/ddb-explorer` from your terminal.
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

Favorites are stored in `ddb-explorer/preferences.json` under the system's config
directory: `$XDG_CONFIG_HOME` (or `~/.config`) on Linux, and
`~/Library/Application Support` on macOS. Terminal favorites are shared across
terminal AWS sessions. Favorite changes are merged under a file lock so multiple
app instances can save safely. History lives under the system's local data directory in
`ddb-explorer/history` (`$XDG_DATA_HOME` or `~/.local/share` on Linux,
`~/Library/Application Support` on macOS). Item contents are saved only when you
choose **Save snapshot**; credentials are never saved. History entries can be
deleted individually, all at once, or when older than 30 days.

## Development

```sh
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
```

`src/main.rs` starts the app, `src/app.rs` holds the explorer view and state,
`src/browser/` separates query forms, results, and testable state transitions.
`src/dynamodb/` owns the shared AWS runtime and clients; its `json.rs` adapter
contains the plain JSON conversion. Shared styling lives in `src/ui/`.

Released under the [MIT license](LICENSE).
