# Backup and Restore Checklist

Status: procedure only, not a completed backup or recovery exercise.
Applies to the running `trade_service`, `copy_trading`, and their separate
SQLite databases. Do not commit archives, `.env` files, passphrases, master
keys, or wallet exports to Git.

## Inventory and capacity gate

| Data | Current local path | Role |
| --- | --- | --- |
| Copy Trading DB | `copy_trading/data/copy_trading.sqlite` | Telegram users, wallet references, strategies, orders, intents, signals |
| Trade DB | `trade_service/data/trades.sqlite` | Durable trade reservations and outcomes |
| Wallet DB | `trade_service/data/wallets.sqlite` | Encrypted wallet secrets and wallet IDs |
| Conversion DB | `trade_service/data/conversions.sqlite` | Conversion reservations and quotes, when enabled |

At the 2026-11-12 check, Copy Trading was about 17 GiB while this host had
only about 4.4 GiB available. Do not create its snapshot or run `VACUUM` on
this filesystem. First provision a restricted external/mounted destination
with room for the full online snapshot, encryption output, and a separate
restore copy. Check actual free space before each operation.

## Before capture

1. Record the source binary hashes, `Cargo.lock` hashes, migration files,
   SQLite file sizes, `PRAGMA page_count`, `freelist_count`, and current
   service status without printing credentials.
2. Inventory the separate Copy Trading `MASTER_KEY_HEX` and Trade Service
   wallet database passphrase file. Keep them in a second secure channel,
   separate from the encrypted database archive. Record file permissions and
   who can decrypt. Do not include them in a public repository or log.
3. For each database, use SQLite's online backup API to produce a consistent
   snapshot while the service runs. Do not copy the main `.sqlite` file alone
   while WAL writes are active. Do not rely on a compressed raw-file copy as
   an online consistency guarantee.
4. Encrypt each snapshot on the restricted destination, record SHA-256 of the
   encrypted artifact, and verify that an authorized operator can decrypt it.
   Remove plaintext staging files only after the encrypted artifact and a
   restored copy have passed integrity checks. Set restrictive permissions.
5. Keep a versioned copy of the matching binaries, configuration template,
   migrations, and this checklist. Keep secrets out of the source archive.

## Isolated restore drill

1. Restore to a host or volume without production listeners or Live tokens.
   Decrypt into a private directory and run `PRAGMA integrity_check` on each
   database. Record table counts for wallets, strategies, orders, trade
   reservations, and `unknown`/in-flight records without exporting secrets.
2. Compare the public-key-to-wallet-ID map between source and restored wallet
   database. Validate wallet decryption with the separate passphrase and
   Copy Trading key in an isolated process; do not print private keys.
3. Start the matching Trade Service binary on loopback in an isolated network
   namespace or host with no route to mainnet broadcast endpoints. Use a test
   API token and mock/read-only RPC services; do not load a production trade
   token or Telegram token. There is currently no general Trade Service
   `TRADE_SUBMISSION_ENABLED` switch: `CONVERSION_LIVE_ENABLED=false` protects
   conversions only. Verify `/health`, authenticated `/ready`,
   `trade_persistence_mode=durable`, database health, and wallet count.
4. Start Copy Trading against the isolated Trade Service and restored DB only
   after the trade endpoint is ready. Verify order and strategy counts and
   read-only status lookup. Do not auto-replay `unknown` orders.
5. Test a rollback of both binaries and databases in isolation. Record elapsed
   recovery time, exact artifact hashes, and any migration incompatibility.

## Acceptance and stop conditions

- Success requires a current off-host encrypted artifact, separately held
  decryption material, integrity checks, matching wallet public-key mapping,
  a loopback restore boot, and an operator-readable recovery record.
- Stop if destination capacity is insufficient, decryption material is
  missing, integrity check fails, wallet mapping changes unexpectedly, or
  any restore process could reach production signing/broadcast endpoints.
- A local binary copy or an unverified archive does not satisfy the P0
  recoverability gate. Do not compact or delete the 17 GiB source database
  before the drill succeeds.
