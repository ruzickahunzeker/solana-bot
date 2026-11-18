# NP-01 Pump.fun ordinary SDK account evidence

Scope: offline proof for the ordinary Pump.fun SDK path, not the paused PumpAPI
transaction adapter. Only the test build exposes the new decoder. Existing
parameter loading, route admission, SDK builders, API and journal are untouched.

The locked SDK decodes the curve after the first eight bytes without checking
its owner or discriminator. Its optional sharing lookup can return `None` on
an RPC error or a malformed account. Those results are not proof of absence.

The new decoder binds the curve PDA, base mint, Global and fee sharing PDA to
one ordered successful snapshot, rejects role/key/count/slot mismatches, and
checks the returned owners, state, size and known prefixes. It reuses the locked
SDK's curve decoder for the legacy 75-byte body or modern 107-byte prefix,
without trusting appended fee fields. A raw native/default quote maps to WSOL;
an actual non-native quote must match exactly. Mint validation covers SPL and
Token-2022 base layout, not epoch, fee, hook or extension approval. Global's
recipient is an observation, not authorization. Only an explicit null sharing
entry is recorded as absent; a present prefix binds the mint and canonical
PDA bump, keeps the raw status, or fails decoding. The pinned IDL orders the
three bytes as `bump`, `version`, `status`; an offline test checks the field
order, discriminator and seeds against that IDL. Snapshot values are
positional and trust the reader to bind them to
its actual request. RPC failure must not be turned into a successful snapshot.

Six decoder and IDL tests cover legacy/current quote layouts, canonical roles
and context, corrupted identities/data, base mint and Global, and
absent/present/malformed sharing state. The four-key test reader uses one processed/base64
`getMultipleAccounts` request with nonzero `minContextSlot`, the existing
parameter semaphore and shared endpoint cooldown. Its deadline cannot exceed
the caller's deadline or two seconds. It does not select another endpoint or
turn transport failure into a successful snapshot. Three loopback tests cover
the exact request, stale/truncated/malformed replies and transport failures,
429 cooldown, cancellation and permit release. The fixtures use generated
public data, not production accounts. Dedicated source-only CI step:
`pump-curve-evidence`.

The native SOL V1 funding model checks eight ordered values from one bank:
the four original roles, canonical curve and user base ATAs, the system-owned
wallet, and Clock. It rejects changed mint program, stale/mismatched Clock,
bad vault authorities, required missing extensions, and curve token balance
below the reported real reserve. Buy can observe an absent output ATA; sell
requires the input ATA but does not infer a positive trade size. The original
read helpers keep Token-2022 fee and hook validation bound to the captured
epoch. Four funding decoder tests cover the role bindings and negative
cases, including an epoch-bound fee change. The native evidence reader sends
all eight keys in one request using the same helper, semaphore, endpoint and
deadline as the four-key reader. Its separate loopback test verifies exact
request order, matching and stale/mismatched Clock contexts, one request and
no fallback. No production consumer calls these readers, and the evidence
does not establish executable fees, output or final transaction validity.

The budget model keeps direction, nonzero integer spend, checked ordinary sell
percentage/floor and separate ATA rent/network fee/tip/other debit ceilings.
Only snapshot native balance can fund those costs: no sell proceeds or future
close refund is credited. The ceilings are supplied test bounds, not proven
quotes, and this model grants no signing authority. Three tests cover exact
balance, absent/existing ATA, direction mismatch, rounding and overflow.

Seventeen Rust tests ran in the focused CI step (six decoder/IDL, four loopback,
four native funding decoder and three budget tests). Commit `be136c0` /
[Actions 44297259103](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44297259103)
completed successfully on its first attempt. Artifact `14427210893` was downloaded
and independently checked: full concurrent 2009 (365.26s), serial 2009 (707.51s),
all parent 1992 names retained with exactly 17 additions, focused 17, HTTP 208,
116 ordered zero-exit steps, 62 types/189 raw field rejections, source and locked
dependency identity. This does not validate any unpublished later source.

Next: validate quote mint/account extensions against bounded bank evidence, then
transaction policy, including min-out, fees, account permissions and the final
unsigned message. No production consumer is wired to the fixed read. Do not
grant a signer or advertise new trading support based on these account prefixes.
The locked Pump V2 builder selects a legacy SPL Token program for its quote
side. A decoded custom quote mint alone therefore cannot authorize a
Token-2022 quote, regardless of its decimals or curve identity. The vendored
holder/route bank and quote fixtures pin instruction wires or derived amounts,
not original account bytes or a fresh oracle. See [unsigned candidate gates](PUMP_SDK_UNSIGNED_CANDIDATE_GATE.md).
