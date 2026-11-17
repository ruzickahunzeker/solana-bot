# Small-VM Development Verification

Date: 2026-11-15, Asia/Shanghai. This changes development verification only, not
SDK versions, public APIs, release behavior, or production service configuration.
The continuous optimization goal was paused during the diagnostic request and
has since been explicitly resumed. That does not authorize repeating the failed
large test build or changing production resources.

## Observed Evidence

- Later acceptance now runs in the user-authorized source-only GitHub Actions
  job, not on this VM. Commit `3f37b3e`, run `43673491613`, passed 1,344 tests
  both concurrently and serially; the artifact was downloaded and checked.
  Newer source changes still need their own run. The diagnostic failures below
  remain historical evidence and do not authorize another large local link.
- This host has about 3.6 GiB RAM, two CPU cores, no swap, and about 18 GiB disk free.
- Boot records show restarts at 08:48 and 09:05 on 2026-11-15. Old tool sessions
  and Cargo processes were gone when inspected; they were not still waiting on
  a Cargo lock. No OOM record explaining those earlier restarts was found. The
  cause of those restarts is not established.
- One resource-limited `cargo check --tests -j 1` passed in about 45 seconds with
  maximum RSS 1,612,596 KiB. This is already substantial on this VM. A missing
  session cannot be treated as a completed check.
- During diagnosis SDK candidate code was unverified. Current source acceptance
  is recorded separately above and in GITHUB_ACTIONS_VALIDATION.md. A type check
  never substitutes for runtime tests or rollout acceptance; do not apply an
  earlier passing count to later source changes.
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

On this VM stop after check/bin-check. Focused tests still link the entire
large binary; use the authorized GitHub Actions job for focused/full/serial
execution. The scope example is not permission to repeat local linking after
the recorded resource failures. Do not remove assertions or grant Live access
to work around a build limitation.

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

Historical diagnostic evidence: type-check passed; runner shell regression tests
passed; the focused Rust tests remain unverified (two memory-limited builds and
one bounded timeout in that diagnostic window). Later cloud acceptance is
recorded above. The released binary and Cargo.lock were not changed by diagnosis.

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

## Compiler Type Boundaries (NP-03 / TS-09)

The aggregator baseline/simulation batch is source wired, not Rust-accepted yet.
Its owned unsigned plan and borrowed read session add two selected concrete types:
37 types / 114 scoped fields including outside alias. The fixed static_lifetime
probe flag permits only true and emits the literal 'static, never supplied Rust;
two additional Python regressions (12 total) cover it and injection rejection.
Expected full suites retain all 1729 names and add exactly 20 (1749), 72 steps.
Do not reuse the accepted 108-field run below or retry local service Cargo after
the resource failure. See AGGREGATOR_SIMULATION_SHARED_READS.md.

The aggregator mint/ALT batch is independently accepted: 10c5b4b / Actions
44164727015 / artifact 14366377330 passes complete concurrent/serial suites each
1729 (212.62/387.94 seconds), all 1713 parent names retained and exactly 16 added.
Runtime 8 / SDK policy 5 / real HTTP 3, all 69 steps and original/restored checks
pass; raw diagnostic spans independently verify the same 35-type/108-field gate
on this source. All 1090 snapshot blobs/modes (13,341,064 bytes) and manifest/lock/
vendor-manifest hashes match; main/index/frozen history remain unchanged. The
first source 43a17e0 failed type checking because pinned UiAccount has no decode
method; original Base64/owner/size mapping and negatives fix it without dependency
changes. Do not rerun either terminal CI, trigger documentation-only CI, retry
local service Cargo after the 2200 MiB failure or deploy. See
AGGREGATOR_ACCOUNT_SHARED_READS.md for scope and remaining simulation/other gates.

The SOL-hop read batch adds SolHopReadSession: 35 selected types / 108 fields
including outside alias. Source 17bc3be / Actions 44156380200 / artifact 14362832442
independently passed original/restored checks, all 108 raw field spans and both
complete 1713-test suites (194.93/355.03 seconds; 66 steps exit zero). All 1693
parent names remain, exactly 20 added; scope 8 / SDK cache 10 / real HTTP 2 and
parameter session 28 pass. All snapshot blobs/modes match (1084 files,
13,296,707 bytes), identity/manifest/lock/vendor-manifest hashes match. Main/index
and frozen history remain unchanged. The exact old serial child-stderr format
and two-positive/seven-negative audit regressions also pass. No local Cargo retry
or production action; do not reuse the 105-field run below, rerun terminal CI,
start documentation-only CI or deploy this source as part of verification.

The ordinary-discovery batch adds DiscoveryReadSession: 34 selected types /
105 scoped fields including outside alias. Source cde04bd / Actions 44146511097 /
artifact 14359025763 independently passed original/restored checks, all 105 raw
field spans, and both complete 1693-test suites (63 steps exit zero). All 1675
parent names remain, exactly 18 added. Scope 8 / loader 8 / real HTTP 2 passed;
three old cancellation fixtures now assert the exact capacities of all eight
classes, not a blanket one-slot assumption or a nonzero check. One serial child
stderr split was strictly associated with its exact name and immediate ok;
two positive/seven negative local audit-format cases passed. Do not reuse older
102-field evidence, retry local Cargo after the resource failure, or treat read
configuration as signing authority. No deployment or real RPC proof is implied.

The 2026-11-18 protocol-parameter batch adds ParameterReadSession to the same
policy: 33 selected types / 102 scoped fields including the outside alias.
Source 1792c51 / Actions 44133854054 / artifact 14351589608 independently passed
all 102 raw-span rejections, original/restored checks and both complete 1675-test
suites (60 steps exit zero). All 1652 parent names remain, exactly 23 added,
including the isolated helper. Cold-success/fault loader witnesses both ran.
This is not acceptance by the older 99-field run, deployment or real RPC proof.
The bounded local check reached MemoryMax=2200M (SIGKILL, 50 seconds,
RSS 2,273,960 KiB; target/dev-checks/20261117T162617Z-1019411). Do not retry
local Cargo or raise the cap; use the authorized source-only CI for this batch.

The 2026-11-17 controlled-buy batch adds the private DammBuyCandidate and
DammBuySignedCandidate: 32 selected types and 99 scoped fields including the
outside-alias probe. Source 882e514 / Actions 44109233896 / artifact 14340653473
passed original/restored checks, all 99 distinct raw-span rejections and both
complete 1638-test suites (56 steps exit=0). Names retain all 1621 parent tests
and add exactly 17. The earlier 30-type/93-field evidence below remains historical.
This is source-only acceptance, not deployment, real-node/provider validation or
proof that all dependency/capability flows are covered.

The 2026-11-16 buy-message batch adds DammBuyQuote and DammBuyMessage to the
same policy: 30 selected types and 93 scoped fields including the outside-alias
probe. Commit ec1d2c9 / Actions 43773807696 / artifact 14185728930 passed the
original/restored compiler checks, all 93 distinct field rejections and both
complete 1,583-test suites. Raw compiler spans and test names were independently
verified; the accepted 28-type/87-field parent evidence below was not reused.
The adapter-specific nominal objects are not new Application signing authority.

The NP-03 task-lifecycle batch added TradeTasks and DispatchTicket to the selected
concrete-runtime denial policy (28 selected types; 87 scoped fields including the
outside-alias probe). Commit c6f2745 / Actions 43718342603 / artifact 14163942665
passed original/restored checks, all 87 distinct field rejections, the 17 new
lifecycle contracts and both complete 1,498-test suites (48 steps exit=0).
The artifact identity, raw test-name sets and raw compiler spans were independently
verified; earlier 81-field runs remain historical, not evidence for 87.

The compiler gate is a development check, not a runtime permission or a new
task. Application, Domain and Protocol production modules forbid
`clippy::disallowed_types`. The current JSON fixture lists 35 selected SDK,
chain, RPC, SQL/HTTP and concrete runtime types; it is not a claim that every
possible dependency or capability leak is covered.

`scripts/check_dependency_boundaries.py` is restricted to the isolated GitHub
job with no runtime inputs. It generates a private Clippy configuration under
ignored target and performs three metadata checks: original source succeeds;
controlled forbidden fields in all three layers fail specifically for the type
boundary; restored original source succeeds. Every field needs its own compiler
diagnostic span. An unrelated compiler error or a missing rejection fails the
gate, rather than counting any nonzero compiler exit as proof.

Clippy alone misses a `type` alias defined outside the guarded layer. The small
`tools/type-boundary-inventory` development-only crate uses the pinned `syn`
parser to collect outside-layer aliases from the service module tree, including
inline modules; comments/strings are not parsed as Rust declarations. The gate
also adds an outside alias in the CI root and repeats inventory, so the extra
three scoped rejection assertions test the actual parser-to-compiler connection.
All 81 negative fields passed in e3be442 / Actions 43711006653; raw diagnostic
spans were independently checked, including all three outside-alias fields.
Guarded layers' own project aliases are not banned; associated trait types are
not mistaken for module aliases. Explicit production `#[path]` modules require
review instead of silently accepting an incorrect module inventory.

All four temporarily changed CI source files are restored byte-for-byte through
`finally`, including partial setup/validation exceptions; SIGTERM interrupts the
owned compiler group before restoration. Nothing is mutated in the trading VM.
Compiler logs, not source snapshots, databases or binaries, enter artifacts.
The workflow installs Clippy for the already pinned Rust 1.95.0, never upgrades
the SDK. The helper has its own manifest/lock and no runtime SDK dependency;
the service manifest/lock/vendor stay unchanged.

Local evidence: ten Python policy/restoration/metadata-only alias regressions
and seven workflow tests passed. The helper's isolated 512 MiB `cargo check`
passed in 5.66 seconds/RSS 232,864 KiB; no service test binary was linked.
The helper's three Rust tests, production/81-field/restored checks and complete
concurrent/serial 1,473-test suites passed in e3be442; artifact 14161032189 was
downloaded and verified (44 steps exit=0). Prior 26445c5 did not contain this
guard. Macro-generated interfaces, opaque capability/value
flows, API-specific rules and remaining dependency directions retain their
original review/acceptance requirements; this is not TS-09 completion.

First cloud run 8bd7796 / Actions 43710108477 failed before the production
boundary check: cargo-clippy did not forward the global `cargo --config ...`
argument to its cargo-check child, which then could not resolve the locked zap
Git source offline. All preceding checks and the helper's three Rust tests
passed; the 81 negatives and complete service suites did not run. The command
now passes `--config` after `clippy`. A dependency-free temporary Cargo fixture
proves the child receives that configuration by requiring its sentinel invalid
target error. This is metadata-only, has no SDK or runtime inputs, and does not
relax offline mode, the lockfile, source provenance or negative assertions.
The corrected production/negative/restoration path is accepted in e3be442;
the failed first run is terminal and must not be restarted.
