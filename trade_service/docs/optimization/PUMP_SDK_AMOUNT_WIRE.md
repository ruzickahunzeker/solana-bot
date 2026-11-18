# NP-01 Pump.fun amount-wire evidence

This ordinary-SDK evidence step follows the account/funding batch `be136c0`.
It remains in the test-only adapter module. The parent's result cannot validate
this later source; this step's own accepted validation is recorded below.

`PumpWireAmounts` reads only the locked SDK's six known discriminator/length
profiles. Native V1 buys have the SDK's trailing `track_volume` bool. The V2
buys and both sells have only two u64 fields. A future/IDL `partial_fill` byte,
unknown discriminator, zero amount/minimum, truncation or malformed flag is
rejected instead of assigned implicit semantics.

The three semantic variants are separate: exact-input buy (spend and minimum),
exact-output buy (output and maximum spend), and exact-input sell (token amount
and minimum quote). The frozen exact-input check cannot accept an exact-output
buy with superficially similar numbers.

Three amount tests call the actual locked SDK `PumpFunInstructionBuilder` for
native SOL, explicit WSOL and USDC, with native cashback on/off and fixed/nonfixed
buy output. Separate sell cases preserve exact input and encoded minimum; values
1 or u64::MAX are ABI fixtures, not executable slippage recommendations. The
mutation test covers damaged input and unsupported trailing bytes. The SDK oracle
uses the same public deterministic, unfunded test identity pattern as DAMM:
no RPC, sender, middleware, account setup/close, executor or signing call.

The bank snapshot now retains its original eight public account keys. A native
V1-only role inspector compares Global, Mint, curve, curve ATA, user ATA, payer
and base token program with those exact bank roles and expected privileges. It
checks unique critical-role occurrences and permits only the payer signer;
exact-output buys and V2 instructions cannot pass this initial profile. It
also binds the native buy `track_volume` byte to the bank's cashback flag, and
checks fixed program/fee-config/event roles and canonical secondary curve and
volume PDA addresses/privileges. It does not approve fee or creator destinations
or establish the on-chain state of those additional accounts. Two more real SDK tests
cover valid buys/sells and twelve mutations each (keys/flags/aliasing/program/
count/volume), plus the
SDK's `pump` mint-suffix Token-2022 override against a legacy SPL-owned mint.
There are five new tests in this successor, separate from the seventeen
previously accepted account/funding tests.

Commit `9bb5330` /
[Actions 44303096331](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44303096331)
completed successfully on its first attempt. Artifact `14429825256` independently
matched source and dependency identity, 116 ordered zero-exit steps, full concurrent
2014 (372.34s), serial 2014 (709.76s), all parent 2009 names retained with exactly
five additions, Pump focused 22, HTTP 208 and 62 types/189 compiler field rejections.
Original snapshot readers/decoder and old tests were independently reconstructed.
The result applies to this immutable commit, not later destination-model source.

This proves neither min-out quote correctness nor fee destination, all privileges,
Token extension compatibility, final compiled message, or deployed-program ABI
acceptance. Random SDK fee destinations are not frozen or authorized by these
amount-only tests. Actual controlled execution still requires all gates in
[the unsigned candidate work](PUMP_SDK_UNSIGNED_CANDIDATE_GATE.md).
