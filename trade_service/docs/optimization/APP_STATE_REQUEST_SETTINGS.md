# AppState Request Settings

Continues the original NP-03/TS-09-A `AppState` ownership work. Production
wiring and three test witnesses passed independent offline CI at `839cd85`.
This does not replace the trade store, claim authority, cache, worker or any
existing Application port.

## Boundary

Previously, production request state kept `Arc<Config>`, including startup
wallet inputs, provider settings, storage configuration and runtime construction
options. The new request state retains only:

| Consumer | Required settings |
| --- | --- |
| HTTP auth | trade token, client IP list, admin mode/token |
| Ordinary admission and execution | existing `TradeIntakeSettings` and `OrdinaryExecutionSettings` projections |
| Ordinary response | existing response wait in milliseconds |
| Pools and capabilities | existing `TradeValidationSettings.via_sol_enabled` |
| Native transfer and WSOL submission | existing broadcast and feature policies |

Produce one immutable `RequestSettings` during bootstrap. Request state carries
one shared handle; it does not retain startup `Config` or gain another runtime
source. Only the required HTTP auth values are copied once; all request
authentication continues to borrow those values. Do not log or serialize this
settings record. Ownership separation does not guarantee secret zeroization or
remove credentials from their actual wallet storage, registry or client owners.

Keep the `Config` parser, startup resource initialization, default values and
shutdown settings unchanged. `main` can extract its existing shutdown deadlines
and stop retaining the full startup config after the listening log. There is no
reload behavior, new configuration key or new privilege. Preserve auth and
validation order, capability payloads, trade identity/hash, amount defaults,
receipt wait behavior, and every table/codec unchanged.

## Fixtures And Acceptance

Keep test-only `Config` for the old decode/hash/default oracles and resource
fixture identity assertions. Production handlers and executor must call the
same narrow settings in test binaries; do not use `cfg(test)` to read the full
config inside those getters. Convert the request/execute fixture mutations to
an explicit test-only reprojection helper, preserving original assertions and
one snapshot per cloned state. Record plain-config mutations used only for
old oracles separately rather than silently altering their expected paths.

Direct witnesses verify that the narrow settings survive release of the source
`Arc<Config>` and that the five flags, broadcast and all scalar defaults project
without coupling. An actual HTTP fixture must verify that the router uses the
same projection and that updating a test fixture cannot mutate earlier clones.
Require existing auth/HTTP/intake/execution/persistence suites plus the new tests
and an independent compiler boundary check. The credential-bearing projection
must not enter Application, Domain or Protocol; each gets only its applicable
policy or existing port. Expect three added tests, retention of every broadcast
baseline test, and three extra type-rejection fields. No local service Cargo check/link,
release build, credential access, production settings inspection, SDK change,
service restart, deployment or trade is part of this work.

The first source-only run [44267836123](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44267836123)
at `1453907` compiled successfully and passed 77 validation steps, including the
177-field boundary gate. Its sole HTTP failure was the new capability assertion:
two independent payload generations differed by one millisecond in
`generated_at_ms`, with the rest of the response identical and all 206 prior
HTTP test names passing. Artifact `14410854771` retains that evidence. The
successor checks the timestamp against the request interval and compares every
other field unchanged. It changes no production code and must pass its own
CI; the stopped run did not execute the full suites and will not be rerun.

The corrected [run 44272714261](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44272714261)
at `839cd85bdbd520f06df0cde0bd0448642427286c` passed on its first attempt.
Artifact `14415756815` independently verifies 109 ordered steps, 207 HTTP
contracts, all three focused witnesses, and matching concurrent/serial names
(1,984 each, 383.21s and 729.06s). All 1,981 accepted broadcast baseline names
remain. The 58-type policy produced all 177 intended compiler rejection fields
and clean original/restored builds. The correction changed no production code.
No SDK change or runtime action was taken.
