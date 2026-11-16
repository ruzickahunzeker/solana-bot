# Original SDK Journal Record Boundary

Date: 2026-11-16. Continues NP-01 in main plan section 3.3; no new task,
authority, SDK dependency, schema or execution profile.

## Typed Data, Original Journal

`domain/sdk_prepared_record.rs::SdkPreparedRecord` holds either the existing
`SdkCandidateMetadata` (SellV1) or `DammBuyCandidateRecord` (BuyV1). Application
journal/backend ports, prepared readback and the linear signing permit now use
this project-owned type. The original sell candidate owns one such record;
there is no second record ledger or copied execution owner.

Encoding delegates to the selected payload without an enum envelope. Historical
SellV1 JSON bytes, profile, validation and message digest stay unchanged. Decoding
checks the 65,536-byte input limit, reads the explicit version/profile pair, then
invokes only that strict payload decoder. Unknown/missing/duplicate discriminators,
cross-direction tags, duplicate/unknown payload fields, trailing JSON and invalid
message size/digest reject. There is no permissive decoder fallback or Value-based
duplicate-key normalization. Original persistence still requires canonical JSON
equality, the nominal protocol/message reconstruction and original intent binding.

The buy codec is now used in production by this common decoder. Being decodable
does not make a buy record eligible to be written or signed. The original claim
and origin scope remain direct Durable Live DAMM **sell only**. Seller-specific
code must use the fallible `sell()` accessor; no buyer is treated as a fake sell
percentage, and no panic or default metadata is used to select a direction.

`prepare_sdk_candidate` and `begin_sdk_signing` reject unsupported direction before
opening a SQL transaction. Existing signing CAS, owner binding and independent
readback remain required. Seller reconstruction, signed-evidence validation and
recovery also reject BuyV1 explicitly. A corrupt nominal buy row is not a recovered
execution owner or signer permission.

## Verification Scope

- Legacy sell provider/mode/percentage matrix preserves JSON bytes through the
  common codec; actual isolated journal storage also preserves the legacy bytes.
- Real unsigned buy fixture and existing asset/mode/ATA/provider matrix round-trip
  through the common codec without acquiring any key or refreshing evidence.
- Both profiles reject malformed/disguised tags, duplicate/unknown payload fields,
  bad digests, size bounds and trailing data.
- A closed temporary pool proves wrong-direction prepare/signing rejects before
  database access. Separate temporary-row snapshots prove no candidate replacement
  or signing-CAS advance; injected corrupt rows reject readback and recovery without
  altering the original owner.
- The original Application controller rejects a buy record before journal prepare
  and any signer/send access; its existing pre-sign terminal bookkeeping is retained.
  A nominal record cannot create or replace the linear sell permit.

These are source/offline contracts, not production database or chain tests.
Current independent validation results belong in `EXECUTION_STATUS.md`; parent
CI results do not validate this batch.

## Remaining Original NP-01 Work

Buy origin/owner admission, fresh runtime evidence authorization, prepared/signed
bytes, controlled signer, frozen direction sender, recovery/replay and observation
scan must be connected together before the original buy claim is opened. The old
seller schema and rejection contracts must remain compatible. This type migration
alone neither enables buying through the controlled journal nor fixes opaque SDK
worker cancellation/expiry. Other protocols/backends/modes, shared RPC budgets,
real provider validation and deployment gates remain unfinished. PumpAPI integration
stays paused; no SDK upgrade or production action is included.
