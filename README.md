# Solana Bot

Solana trading stack containing the execution service, PumpAPI data bridge, and Telegram copy-trading application.

## Components

- `trade_service/`: wallet custody, pool resolution, transaction construction, submission, persistence, and the Public Trade API.
- `pumpapi_zmq_bridge/`: PumpAPI WebSocket consumer and local ZMQ publisher. It does not hold keys or submit trades.
- `copy_trading/`: Telegram wallet management, manual and limit trading panels, and PumpAPI-driven copy trading. Live execution is delegated to `trade_service`.

## Runtime Flow

```text
PumpAPI WebSocket
      |
      v
pumpapi_zmq_bridge --ZMQ--> copy_trading --HTTP--> trade_service --RPC/MEV--> Solana
```

`trade_service` is the only component that stores encrypted private keys and signs transactions. `copy_trading` stores external wallet IDs and sends explicit execution parameters to the Trade API.

## Start Order

1. Configure and start `trade_service`.
2. Start `pumpapi_zmq_bridge` on `tcp://127.0.0.1:8939`.
3. Configure and start `copy_trading`.

Each component has its own README and `.env.example`. Keep real `.env` files, wallet databases, passphrases, logs, and build output outside Git.

## Safety

- Begin with Dry-run or simulation mode.
- Use small amounts for the first Live transaction.
- Keep API tokens, RPC credentials, Telegram tokens, wallet passphrases, and private keys out of Git.
- Back up encrypted wallet databases and their passphrases separately.
