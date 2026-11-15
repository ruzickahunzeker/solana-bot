# Small-VM Development Verification

Date: 2026-11-15, Asia/Shanghai. This changes development verification only, not
SDK versions, public APIs, release behavior, or production service configuration.
The continuous optimization goal was paused during the diagnostic request and
has since been explicitly resumed. That does not authorize repeating the failed
large test build or changing production resources.

## Observed Evidence

- This host has about 3.6 GiB RAM, two CPU cores, no swap, and about 18 GiB disk free.
- Boot records show restarts at 08:48 and 09:05 on 2026-11-15. Old tool sessions
  and Cargo processes were gone when inspected; they were not still waiting on
  a Cargo lock. No OOM record explaining those earlier restarts was found. The
  cause of those restarts is not established.
- One resource-limited `cargo check --tests -j 1` passed in about 45 seconds with
  maximum RSS 1,612,596 KiB. This is already substantial on this VM. A missing
  session cannot be treated as a completed check.
- SDK candidate code is an uncompleted development checkpoint. A successful
  type check does not replace focused/full tests, production checks or rollout
  acceptance. Do not reuse the older 1,304-test result for the current draft.
- Two diagnostic builds exceeded an isolated 1800 MiB cgroup limit before any
  test ran. The first kernel record explicitly identifies this development
  scope, not a system-wide OOM. RSS reached approximately 1.7 GiB. Removing test
  debug information and incremental compilation alone did not fit that cap.
  These newly generated diagnostic OOM records do not establish the cause of
  earlier host reboots.
- A Rust test filter only selects execution: it still compiles the service's
  entire test binary, containing over 1,300 tests. Repeatedly launching a narrow
  filter does not avoid this compile-time resource requirement.

## Changes

`scripts/dev_check.sh` provides explicit check/bin-check/focused/full/serial/status modes,
one nonblocking lock, 15-second heartbeats, timeouts, exit status, boot identity,
manifest fingerprints and `/usr/bin/time -v` resource statistics. Logs live in
ignored `target/dev-checks/`; no `.env`, wallet file or production database is read
by the runner. It never starts a service or submits a transaction.

The service package's test debug information and incremental compilation are
disabled, with 256 codegen units. Dependency profiles,
release profiles, Cargo.lock and vendor are unchanged by this diagnostic change.
This avoids changing global RUSTFLAGS and rebuilding the entire SDK dependency
tree. Test assertions are not disabled.

The script alone limits jobs, test threads, priority and time, **not RAM**. Use a
temporary systemd scope for the hard memory/CPU limit on this VM:

```bash
cd /root/http/sol-bot/trade_service
env TRADE_DEV_TIMEOUT_SECONDS=300 systemd-run --scope --quiet \
  --unit="solbot-dev-check-$(date -u +%Y%m%dT%H%M%S)" \
  --property=MemoryMax=2200M \
  --property=MemorySwapMax=0 \
  --property=CPUQuota=100% \
  bash scripts/dev_check.sh check
```

After the type check succeeds, use the same scope command with
`bash scripts/dev_check.sh focused sdk_candidate`. Replace only the test filter
for another focused group. Full/serial suites are milestone checks; use a
sufficient timeout or a larger development host. Do not retry a memory-killed
compiler without the cap, remove assertions, or grant a route Live permission.

The 2200 MiB cap was selected for a host with about 3.6 GiB physical RAM and
roughly 2.8 GiB available before compiling; it leaves about 1.4 GiB of physical
capacity outside the scope. Reassess headroom when other workloads change. If
this budget cannot compile the test binary, stop local code generation and use
a larger development/CI host rather than raising the cap repeatedly. Prefer
at least 8 GiB for continued development, separate from live trading workloads.
Adding swap or splitting the test harness requires a separate scoped change.

Do not set a low `MemoryHigh` threshold on this swap-less VM: the diagnostic
scope accumulated over 229,000 high-limit events and millions of file-page
refaults while its anonymous compiler working set remained above that line.
This caused reclaim stalls. Raising the soft line from 1800 to 2100 MiB did not
resolve them; the final portion retained the hard cap but removed the soft line.
The same diagnostic run reached its 300-second timeout without starting tests.
No further automatic local compile retry was started. This does not prove that
a fresh 2200 MiB hard-cap-only build will pass: use a larger build environment
for the remaining acceptance rather than treating the changed command as proof.

Final diagnostic evidence: type-check passed; runner shell regression tests
passed; the focused Rust tests remain unverified (two memory-limited builds and
one bounded timeout). The released binary and Cargo.lock hashes remain unchanged.

```bash
bash scripts/dev_check.sh status
bash scripts/test_dev_check.sh
python3 scripts/test_sdk_candidate_migration.py
```

`check` type-checks test code; `bin-check` type-checks the production configuration
without linking a binary. The standalone SQLite test is additional DDL evidence
only, not a replacement for Rust/SQLx/SDK/HTTP or full-suite acceptance.

Normal completion/failure/timeout is recorded separately. If the host or runner
is killed before it can write a result, `status` reports an unconfirmed marker
when no runner holds the lock, and shows reboot identity changes. A lock alone
does not verify a live compiler; inspect the actual process before waiting.
For a `solbot-dev-*.scope` cgroup, before/after memory event counters are saved;
an observed increase in `oom_kill` is reported as `resource_limited`, retaining
the command's original nonzero exit code.

No swap file, permanent systemd configuration, service restart or disk deletion
is performed. The temporary scope applies only to the development command.
