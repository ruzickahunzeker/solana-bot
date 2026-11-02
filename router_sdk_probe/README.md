# Router SDK Probe

This is an isolated compile probe for `sol-trade-router-sdk`.

- The dependency is pinned to commit `de21397aae7836d11124b08b047e390cba3fd46e`.
- It is not linked into `trade_service`.
- It does not load keys, query a wallet, deploy a program, sign, simulate, or broadcast a transaction.
- A successful build only proves the client API and dependency graph can compile in isolation.

The production switch remains Jupiter until a matching Router program is deployed and a
dedicated backend passes protocol-by-protocol simulation and small-value acceptance.
