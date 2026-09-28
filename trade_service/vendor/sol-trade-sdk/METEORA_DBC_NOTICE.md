# Meteora Dynamic Bonding Curve support

The Meteora DBC integration uses the official MIT-licensed Rust program and
quote SDK from:

- https://github.com/MeteoraAg/dynamic-bonding-curve
- Revision: `3b540e94b5b20ba37733de6e25f58522a0cd8961`

The pinned revision supplies the zero-copy account layouts and exact-in quote
math used to construct slippage-protected `swap2` instructions. The transaction
builder and trade-service integration in this repository are project code.
