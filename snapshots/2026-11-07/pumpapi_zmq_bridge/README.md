# PumpAPI ZMQ Bridge

独立的 PumpAPI 数据流桥接服务。它只负责：

- 连接 `wss://stream.pumpapi.io`
- 通过 ZMQ PUB 广播原始 PumpAPI JSON 消息
- 可选广播归一化后的交易候选消息，方便 `trade_service` 或其它客户端消费

它不调用交易 API，不读取钱包私钥，也不下单。

## 依赖

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
```

## 启动

```bash
cd /root/solana/pumpapi_zmq_bridge
.venv/bin/python pumpapi_zmq_bridge.py
```

默认绑定：

```text
tcp://127.0.0.1:8939
```

## 多客户端订阅

订阅原始 PumpAPI JSON：

```bash
./zmq_sub_client.py
```

订阅归一化候选：

```bash
./zmq_sub_client.py normalized
```

可以同时启动多个订阅客户端。

归一化消息会把明确的 `meteora-dbc`、`dynamic-bonding-curve`、`dbc` 标记映射为
`meteora-dbc`。泛称 `meteora` 仍映射为 `meteora-damm-v2`，避免把 DAMM 与 DBC
误认为同一种交易协议。

## 监听指定钱包买入同一 mint

新增 `wallet_watch_recorder.py`，用于订阅本 bridge 广播的 PumpAPI raw event，并保存目标钱包买入记录。

默认监听这两个钱包：

```text
CCCCQCrL6zVjnDeucDzcxJgxAs5ahNmrhw1CDexPhqrd
nya666pQkP3PzWxi7JngU3rRMHuc7zbLK8c8wxQ4qpT
```

第一版逻辑：

- 只处理 `action=buy`。
- 优先使用 PumpAPI 事件里的 `tradersInvolved` 判断钱包是否参与交易；没有该字段时才回退 `txSigner`。
- 命中目标钱包后写入 SQLite 表 `wallet_buy_events`。
- 如果另一个目标钱包在时间窗口内也买过同一个 `mint`，写入 `mint_wallet_matches`。
- 每个命中 `mint` 会继续追踪 10 分钟，记录 `price`、`marketCapQuote`、`marketCapSol` 的最高值到 `mint_10m_highs`。
- 使用唯一键去重，避免 WebSocket 重连或回放重复造成重复入库。

先启动 bridge：

```bash
cd /root/solana/pumpapi_zmq_bridge
./pumpapi_zmq_bridge.py
```

另一个窗口启动钱包监听：

```bash
cd /root/solana/pumpapi_zmq_bridge
./wallet_watch_recorder.py
```

自定义配置：

```bash
WATCH_WALLETS=CCCCQCrL6zVjnDeucDzcxJgxAs5ahNmrhw1CDexPhqrd,nya666pQkP3PzWxi7JngU3rRMHuc7zbLK8c8wxQ4qpT \
WALLET_MATCH_WINDOW_SECONDS=3600 \
HIGH_TRACK_SECONDS=600 \
WALLET_WATCH_DB=/root/solana/pumpapi_zmq_bridge/data/wallet_watch.sqlite \
ZMQ_ENDPOINT=tcp://127.0.0.1:8939 \
./wallet_watch_recorder.py
```

初始化数据库：

```bash
./wallet_watch_recorder.py --init-db
```

回放内置样本验证：

```bash
rm -f /tmp/wallet_watch_test.sqlite
./wallet_watch_recorder.py \
  --db /tmp/wallet_watch_test.sqlite \
  --replay-file fixtures/wallet_watch_sample.jsonl
```

查询最近匹配：

```bash
./wallet_watch_recorder.py --list-matches 20
```

查询最近命中 mint 的 10 分钟最高值：

```bash
./wallet_watch_recorder.py --list-highs 20
```

或直接查 SQLite：

```bash
sqlite3 /root/solana/pumpapi_zmq_bridge/data/wallet_watch.sqlite \
  "select matched_at_ms,mint,first_wallet,second_wallet,first_signature,second_signature from mint_wallet_matches order by matched_at_ms desc limit 20;"
```

查看高点表：

```bash
sqlite3 /root/solana/pumpapi_zmq_bridge/data/wallet_watch.sqlite \
  "select mint,high_market_cap_quote,high_market_cap_quote_signature,high_price,tracking_done from mint_10m_highs order by first_matched_at_ms desc limit 20;"
```

## 配置

```env
PUMPAPI_STREAM_URL=wss://stream.pumpapi.io
ZMQ_BIND=tcp://127.0.0.1:8939
ZMQ_SNDHWM=10000
ZMQ_PUBLISH_NORMALIZED=true
ZMQ_PUBLISH_OBSERVATION=true
PUMPAPI_CAPTURE_LIMIT=0
STONKFUN_ROUTE_FILE=/root/solana/pumpapi_zmq_bridge/data/stonkfun_mints.txt

WATCH_WALLETS=CCCCQCrL6zVjnDeucDzcxJgxAs5ahNmrhw1CDexPhqrd,nya666pQkP3PzWxi7JngU3rRMHuc7zbLK8c8wxQ4qpT
WALLET_MATCH_WINDOW_SECONDS=3600
HIGH_TRACK_SECONDS=600
WALLET_WATCH_DB=/root/solana/pumpapi_zmq_bridge/data/wallet_watch.sqlite
ZMQ_ENDPOINT=tcp://127.0.0.1:8939
```

`STONKFUN_ROUTE_FILE` 保存已由 StonkFun LaunchLab 配置确认的 mint。Bridge 使用该索引把这些 mint 迁移后的 `raydium-cpmm` 事件规范化为 `stonkfun-swap`；文件只包含 mint，每行一个。

`ZMQ_BIND` 支持逗号分隔多个地址：

```bash
ZMQ_BIND=tcp://127.0.0.1:8939,ipc:///tmp/pumpapi-zmq ./pumpapi_zmq_bridge.py
```

## 注意

PumpAPI 官方限制同一侧只保留一个 WebSocket 连接。如果机器上已有其它服务连接 PumpAPI stream，需要停掉那个连接，或让所有下游都从这个 ZMQ bridge 订阅。
