# Trade API Security Hardening

Status: source-level findings as of 2026-11-11, with local host checks on
2026-11-12. No network or credential configuration was changed. Cloud-side
reachability, proxy rules, and tunnel/TLS termination remain unverified.

## Confirmed source behavior

- `trade_service/src/http.rs` routes `/internal/*` through
  `authorize_internal`, which checks the peer address is loopback and then
  applies the ordinary Bearer/IP protection.
- The wallet create, manage, delete, and private-key export routes are under
  `/v1/wallets*` and use `authorize_protected`, as do the trade routes.
- `authorize_protected` checks the configured client IP allowlist and the
  single `LOCAL_TRADE_API_TOKEN`. Source review therefore does not show
  separate trade and wallet-administration credentials.
- `trade_service/src/config.rs` rejects a non-loopback bind without a nonempty
  client-IP allowlist. An allowlist is not a substitute for transport
  encryption or route-level privilege separation.
- On the running host, Trade Service listened on `0.0.0.0:18787`; UFW was
  inactive and the inspected host iptables chain had no rule specific to
  port 18787. The visible primary address was private (`10.9.0.15`). This
  does not prove public reachability or cloud security-group protection.

## Before any new Live rollout

1. On the deployment host, confirm the effective bind address, firewall,
   tunnel/TLS, proxy path allowlist, and which clients can reach each route.
   Record evidence without publishing addresses, secrets, or wallet material.
2. Inventory current callers of wallet-management and export routes. Keep
   those callers functional during migration; do not silently revoke a Bot
   workflow or change the existing token in place.
3. Introduce a separate admin credential or strictly loopback-only management
   listener. The trade credential must not authorize export, create, update,
   or delete. Keep an explicit transition period with audit logs.
4. Apply an upstream path allowlist for trading clients that excludes wallet
   management/export and `/internal/*`; verify direct-to-service access is
   also blocked where expected.
5. Test authorized/unauthorized requests from each actual network path. Verify
   that error bodies, logs, backups, and diagnostics never include private
   keys or full tokens.
6. Rotate credentials only after all callers have been migrated and a rollback
   path has been rehearsed. Retain a local emergency stop and a current
   encrypted recovery backup.

No new protocol or wallet client should be granted Live access until these
checks and the durable persistence gate are verified in the running service.
