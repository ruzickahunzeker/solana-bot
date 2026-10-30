# Bot SSH tunnel deployment

## Topology

- Trade server: `10.9.0.15`, Trade API and encrypted wallet database.
- Bot server: `10.9.0.7`, bot_trading and pumpapi_zmq_bridge.
- Bot HTTP endpoint: `http://127.0.0.1:18788`.
- Tunnel destination on the trade server: `127.0.0.1:18787`.

## Trade server: installed

The dedicated account is `trade-tunnel`. Configuration files:

- `/etc/ssh/sshd_config.d/60-trade-tunnel.conf`
- `/etc/ssh/authorized_keys/trade-tunnel`

Only source `10.9.0.7` may authenticate, using the supplied public key.
Only local TCP forwarding to `127.0.0.1:18787` is allowed. Sessions,
shells, SFTP, remote forwarding, Unix socket forwarding, agent forwarding,
and PTYs are disabled. The public key file is owned by root and is readable
but not writable by the tunnel account. Settings apply only to this account.

Authorized client key fingerprint:

```text
SHA256:AJD/k4Xo3RI0+2NhDeTtqsC/ypqcWT4pnmPN0+lUmVA
```

Trade server ED25519 host key fingerprint (verify on the Bot server):

```text
SHA256:hnOM1R4xGFKz6+GLwDA7Skd6OfdVlq735pJDxAKst5w
```

SSH syntax and effective configuration have been checked, and SSH was
reloaded. The root account's effective settings were unchanged.
The tunnel destination `/health` returned `{"ok":true}`.
An actual tunnel login from the Bot server remains to be tested.

## Bot server: pending

Use the private key corresponding to the supplied public key. The example
below assumes it is `/root/.ssh/trade_tunnel_ed25519`; substitute its actual
path. Keep the private key on the Bot server.

Verify the server host key against the fingerprint above and record it in
the Bot server's known_hosts file before unattended startup. Do not disable
host key checking.

```bash
ssh -N -T \
  -i /root/.ssh/trade_tunnel_ed25519 \
  -o IdentitiesOnly=yes \
  -o BatchMode=yes \
  -o StrictHostKeyChecking=yes \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=15 \
  -o ServerAliveCountMax=3 \
  -L 127.0.0.1:18788:127.0.0.1:18787 \
  trade-tunnel@10.9.0.15
```

Once the tunnel is running, test from another Bot server terminal:

```bash
curl --connect-timeout 3 --max-time 5 http://127.0.0.1:18788/health
```

Run authenticated, read-only checks for `/ready`, `/v1/wallets/manage`, and
`/internal/wallet-balances` with the existing token and correct owner.
HTTP forwarding connects to loopback on the trade server, allowing the
existing internal wallet endpoints to work. No transaction is needed for
these checks. Forward establishment alone does not prove API readiness.

Bot environment:

```ini
TRADE_SERVICE_URL=http://127.0.0.1:18788
PUMPAPI_ZMQ_ENDPOINT=tcp://127.0.0.1:8939
```

Set `TRADE_SERVICE_TOKEN` to the trade server's existing
`LOCAL_TRADE_API_TOKEN` through a private channel; never publish it.

Run the tunnel as a systemd service with `Restart=always` and `RestartSec=3`.
The Bot unit should depend on the tunnel unit and check API health and
readiness at port 18788 before starting. Later tunnel failures still require
normal Bot network-error handling; service ordering only controls startup.

## Bot database migration

Preserve the original SQLite database, app_users.id values, and required
MASTER_KEY_HEX. The wallet owner namespace remains `copy_trading` even if
the deployed directory is named bot_trading. Use SQLite backup or a complete
stopped-database backup, accounting for WAL files.

Before starting the migrated Bot, inspect interrupted orders. Order recovery
can resubmit persisted requests even with stream consumption disabled.
The old Bot must remain stopped while the new instance uses the same token.
Trade server copy_trading, bridge, and recorder services remain stopped.

The SSH tunnel carries the existing API credential. It does not add
per-wallet authorization: the current shared token retains broad wallet
management and export permissions.
