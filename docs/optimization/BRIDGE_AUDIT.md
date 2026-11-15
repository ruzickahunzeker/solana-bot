# PumpAPI Bridge Delivery Audit

Date: 2026-11-12 (Asia/Shanghai). Source audit plus offline tests. The
running Bridge was not restarted or reconfigured for this audit.

## Current contract

- One PumpAPI WebSocket connection feeds the ZeroMQ PUB socket. The raw
  channel is a single frame; observation and normalized use multipart topics.
  `copy_trading` currently consumes the raw stream, so changing its shape
  would break an active consumer.
- Observation messages use schema version 1 and include `event_id`, source
  and receive timestamps, route status, and canonical raw hash. Observation
  JSONL is optional and rotates between the active file and one `.1` file.
  Rotation limits storage but does not establish a replay cursor.
- Normalized messages use schema version 1 and are optionally written to a
  capture file. That capture has no consumer cursor, acknowledgement, or
  bounded retention policy. The raw capture is also append-only without a
  configured size limit.
- ZeroMQ PUB/SUB has no acknowledgement. Slow or disconnected subscribers
  can miss messages; `ZMQ_SNDHWM` only bounds a sender queue. A successful
  `send_multipart` does not prove that Copy Trading persisted an event.

## Identity and completeness risks

- For signed events, the v1 ID is `signature:action:mint:pool_id`. Two distinct
  events in one Solana transaction with the same four fields collide even if
  amounts or inner-instruction positions differ. For unsigned events, a
  canonical JSON hash is used, but those events are observation-only.
- The raw source may not provide an instruction/event index. Appending a
  local arrival counter would distinguish duplicates but would not be stable
  across replay. V2 needs a documented preference order: upstream stable
  index, verified chain instruction index, or explicit `ambiguous` identity
  with a content hash and no Live trigger. Never silently promote a guessed
  index to a chain-unique identity.
- The Bridge does not track source sequence numbers, upstream replay range,
  or a durable normalized delivery offset. PumpAPI disconnects may produce
  an unrecoverable source gap even if the local spool is perfect.

## Compatibility-first implementation sequence

1. Capture sanitized fixtures for multi-event transactions, repeated raw
   messages, missing signatures, and disconnect/reconnect. Check whether
   PumpAPI supplies a stable event or instruction index before choosing a v2
   identity formula.
2. Add a v2 envelope on a new ZMQ topic while leaving raw and v1 topics
   untouched. Include identity quality, source event time, receive time,
   protocol/pool, raw hash, and a Bridge-local monotonic sequence. A local
   sequence detects Bridge-to-consumer gaps but does not prove upstream
   completeness.
3. Add an append-only normalized log with bounded segments, checksums,
   retention, and explicit disk-full behavior. Persist before PUB. Track a
   durable consumer cursor and replay by offset; consumers deduplicate by
   stable event identity and separately report ambiguous identities.
4. Test a slow subscriber, disconnected subscriber, Bridge crash after log
   write/before PUB, consumer crash after processing/before cursor advance,
   full disk, and repeated upstream events. Validate legacy v1 subscribers
   still receive unchanged messages.
5. Run a 24-hour shadow soak with sequence gaps, lag, replay count, duplicate
   count, and disk growth metrics. Restrict Live follow paths if upstream
   gaps cannot be repaired.

## Verification and release gate

`python3 -m unittest discover -p 'test_*.py' -v` passed 10 tests on
2026-11-12. Those tests cover normalization and observation behavior, not
delivery guarantees. No claim of lossless normalized delivery is justified
until replay and gap tests above pass.
