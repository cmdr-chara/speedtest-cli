# DNS subsystem

The v0.4 DNS subsystem treats resolver testing and resolver configuration as separate operations.

## Benchmarking

Built-in resolver profiles are grouped into `fastest`, `privacy`, `security`, `adblock`, `family`, and `all` leagues. Benchmarks use direct DNS-over-UDP/53 probes and rank providers using latency distribution, tail latency, reliability, and stability rather than a single lookup.

The registry includes profiles from Cloudflare, Google Public DNS, Quad9, Control D, AdGuard DNS, CleanBrowsing, OpenDNS, and DNS.SB. Custom resolver IPs can also be tested without adding them to the built-in registry.

## Configuration safety

Persistent DNS writes are deliberately conservative:

1. Inspect the active or explicitly selected interface.
2. Preflight the candidate resolver before changing system state.
3. Save a rollback snapshot of the existing DNS configuration.
4. Apply through the operating system's supported network-management mechanism.
5. Flush the local resolver cache where supported.
6. Verify name resolution through the system resolver after the change.
7. Restore the snapshot automatically if post-change verification fails.

`--dry-run` never writes. `--yes` skips only the CLI confirmation prompt and does not bypass operating-system privilege requirements.

`dns reset` returns an interface to automatic/DHCP-managed DNS. `dns rollback` restores the most recent saved DNS snapshot when the platform backend can reproduce it.

## Platform behavior

- Windows: DNS Client / network-adapter configuration through PowerShell cmdlets.
- macOS: network-service DNS configuration through `networksetup`.
- Linux: persistent automatic configuration is supported when NetworkManager manages the active connection. Unmanaged resolver setups stay read-only instead of editing `/etc/resolv.conf` behind another resolver manager.

The DNS configuration layer must not silently rewrite every interface. The default target is the interface carrying the active route; VPN, VM, container, overlay, and other auxiliary adapters are left alone unless explicitly selected.
