# ferro-wg

A WireGuard VPN client with a terminal UI and swappable cryptographic backends for performance comparison.

Built in Rust. Manages multiple VPN connections, each with its own WireGuard identity. Uses a privileged daemon architecture so the TUI and CLI run without root.

## Quick Start

```bash
# Build and install
git clone https://github.com/scull7/ferro-wg.git
cd ferro-wg
./install.sh

# Import a WireGuard config
ferro-wg import ~/Downloads/BAR_datacenter.conf

# Start the daemon (needs root for TUN devices)
sudo ferro-wg daemon

# In another terminal — connect
ferro-wg up
ferro-wg status
```

## Features

- **Multiple named connections** — each with its own private key, TUN device, and UDP socket
- **Swappable WireGuard backends** — boringtun (Cloudflare), neptun (NordSecurity), gotatun (Mullvad)
- **wg-quick import** — reads standard `.conf` files, supports hostnames
- **Privileged daemon** — creates TUN devices as root, CLI/TUI connects over Unix socket
- **Auto-reload** — import new configs without restarting the daemon
- **Terminal UI** — ratatui-based with Status, Peers, Compare, Config, and Logs tabs
- **Diagnostic tools** — `ferro-wg routes` shows active tunnel routing, `ferro-wg status` warns when a server isn't responding

## Commands

```
ferro-wg import <file.conf>       Import a wg-quick config as a named connection
ferro-wg daemon                   Start the privileged daemon (foreground)
ferro-wg daemon --daemonize       Start the daemon in the background
ferro-wg daemon --stop            Stop the running daemon
ferro-wg up [name]                Bring up all connections (or one by name)
ferro-wg down [name]              Tear down connections
ferro-wg status                   Show connection status with diagnostics
ferro-wg routes                   Show active tunnel routes
ferro-wg tui                      Launch the interactive terminal UI
ferro-wg genkey                   Generate an X25519 keypair
```

## Architecture

```
ferro-wg CLI/TUI          Unix Socket IPC          ferro-wg daemon (root)
(unprivileged)         <====================>       |
                                                    |-- TUN device per connection
  ferro-wg up                                       |-- UDP socket per connection
  ferro-wg status                                   |-- WgBackend packet loop
  ferro-wg tui                                      |-- route/DNS configuration
```

The daemon runs as root (required for creating macOS utun devices) and listens on `/tmp/ferro-wg.sock`. The CLI and TUI communicate over this socket using newline-delimited JSON.

### Crate Structure

| Crate | Purpose |
|---|---|
| `ferro-wg-core` | Backend trait, config parsing, tunnel manager, IPC protocol, daemon server |
| `ferro-wg` | CLI binary and ratatui TUI |
| `ferro-wg-daemon` | Standalone daemon binary (thin wrapper around core) |

### Backend Abstraction

The `WgBackend` trait normalizes the three WireGuard implementations behind a sync, buffer-oriented API:

```rust
pub trait WgBackend: Send {
    fn encapsulate(&mut self, src: &[u8], dst: &mut [u8]) -> PacketAction;
    fn decapsulate(&mut self, src_addr: Option<SocketAddr>, datagram: &[u8], dst: &mut [u8]) -> PacketAction;
    fn initiate_handshake(&mut self, dst: &mut [u8], force: bool) -> PacketAction;
    fn tick(&mut self, dst: &mut [u8]) -> PacketAction;
    fn stats(&self) -> TunnelStats;
    fn reset(&mut self);
    fn backend_name(&self) -> BackendKind;
}
```

Each backend adapter maps the library-specific API to this common interface:

| Backend | Crate | Buffer Model | Maintainer |
|---|---|---|---|
| boringtun | `boringtun 0.7` | Borrowed `&mut [u8]`, V4/V6 split | Cloudflare |
| neptun | `neptun 2.2` (git) | Borrowed `&mut [u8]`, unified `IpAddr` | NordSecurity |
| gotatun | `gotatun 0.5` | Owned `Packet` (bytes crate) | Mullvad |

## Configuration

Config is stored at `~/.config/ferro-wg/config.toml` (macOS: `~/Library/Application Support/ferro-wg/config.toml`).

Each imported wg-quick file becomes a named connection:

```toml
[connections.BAR_datacenter.interface]
private_key = "base64..."
addresses = ["172.31.250.32/32"]

[[connections.BAR_datacenter.peers]]
name = "BAR_datacenter"
public_key = "base64..."
preshared_key = "base64..."
endpoint = "wireguard.vpn.bar.example.com:51821"
allowed_ips = ["10.21.0.0/16"]

[connections.FOO_datacenter.interface]
private_key = "base64..."
addresses = ["172.31.255.18/32"]

[[connections.FOO_datacenter.peers]]
name = "FOO_datacenter"
public_key = "base64..."
endpoint = "wireguard.vpn.foo.example.com:51821"
allowed_ips = ["10.32.0.0/16"]
```

## Troubleshooting

### Server not responding

```
$ ferro-wg status
connection: YOUR_datacenter
  tx: 28140 bytes
  rx: 0 bytes
  warning: sending but not receiving — server may not have this public key
```

The public key shown in `ferro-wg status` must be registered as an allowed peer on the WireGuard server. Share it with your server admin.

### Permission denied

```
$ ferro-wg up
Error: daemon is not running.

Start it with:
  sudo ferro-wg daemon
```

The daemon needs root to create TUN devices. Run it with `sudo`.

### Routes not working

```bash
ferro-wg routes    # show active tunnel routes
netstat -rn        # full system routing table
```

Check that the allowed IPs in your config match the network you're trying to reach.

## Building

Requires Rust 1.88+ (edition 2024).

```bash
# Build everything
cargo build --workspace

# Run tests (93 tests)
cargo test --workspace --features boringtun,neptun,gotatun

# Clippy
cargo clippy --workspace --features boringtun,neptun,gotatun -- -W clippy::pedantic -D warnings
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
