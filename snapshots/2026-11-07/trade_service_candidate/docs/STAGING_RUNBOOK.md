# Candidate 隔离 staging 执行手册

本手册用于隔离 HTTP 检查和只读 Shadow 验收。新增的测试进程使用一次性文件钱包、
临时端口和独立 SQLite；`current` 与 18788 不参与。18887 上的旧版 Jupiter
quote-only Shadow 已于 2026-11-06 完成验收，sampler 已关闭开机自启，证据保存在
`/root/solana/trade_service/evidence/jupiter-shadow-20261105T004653Z/`。
2026-11-07，18887 已更新为 SDK 11 simulate-only 二进制 SHA-256
`405494b7de8759977e0118d36e96732cb82e777786c1eee27798f27dfe80ea7a`。
升级前备份二进制及三份 SQLite，升级后 HTTP smoke、持久化回读、运行进程哈希与
只读 CPMM/USDT→WSOL 卖出决策通过；交易/聚合器报价/提交行数保持 122/0/0。
备份、回滚脚本和结果位于
`/root/solana/trade_service/evidence/sdk6-sdk11-priority-20261107/`。
Bot 的隔离 SDK 模拟开关已启用，主提交关闭；Bot 服务仍 inactive/disabled。
新增 candidate 的预部署运行验收应使用独立端口和数据库，不复用这份已归档的窗口。
窗口结束后，SDK 11 candidate `dd1f6235150a` 已更新到 18887；首次 HTTP smoke、
同库重启后的 HTTP smoke 与 SQLite 回读均通过。旧二进制及三个 staging 数据库的
回滚副本位于 `/root/solana/trade_service/evidence/staging-upgrade-20261106T005806Z/`。
Jupiter sampler 保持 inactive/disabled，`current` 和 18788 未切换。
后续 candidate `a384d52cafce` 已在 2026-11-06 更新到 18887：
`/v1/routes/decide` 对已解析钱包返回 `wallet_pubkey`，供 P4 只读客户端逐次核对。
隔离 HTTP 的已知钱包、缺失钱包和零交易写入检查，以及 18887 正负向请求、两次
HTTP smoke、同库重启回读与运行进程哈希均通过。回滚副本与检查记录在
`/root/solana/trade_service/evidence/staging-wallet-binding-20261106T013817Z/`。
后续 candidate `7e9bb07c4a9d` 已部署到 18887，为 `sdk_verified` 单池与
StonkFun Via-SOL 决策补充实际构建器和链上路线证据。公开 PumpSwap 的隔离 HTTP
正负向通过；部署后真实钱包只读决策返回 `PumpSwapParams / verified_direct_sdk`、
`jupiter_candidate_allowed=false`，交易相关表行数不变。升级前二进制、三份
staging SQLite 备份和运行验收记录位于
`/root/solana/trade_service/evidence/staging-direct-sdk-evidence-20261106T055056Z/`。

源码门槛使用 `scripts/verify_candidate_gates.sh`。脚本固定使用已安装的 Cargo/Rustc
1.99.0，在 `/tmp/candidate-sdk-route-target` 构建，并把当前源码和二进制的哈希写入
`builds/candidate-<binary>-<source>/BUILD_INFO`。它单独验证冻结的 18887 基线和新归档，
不会覆盖 `candidate/target/release/pump-trade-service`。运行中的 18887 仍按原
`BUILD_INFO` 与 `AUDIT_MANIFEST.json` 核对；新源码只按隔离归档核对。

2026-11-06 的 `candidate-f74ea99440aa-310381c76244` 加入 CPMM→CPMM 原生 SDK
两跳预览和 simulate-only 决策。公开 USDC Quote CPMM 池的隔离有资金回归得到目标
代币增量 23,860,003 ≥ 最低到手 23,613,483；交易状态 `simulated`、签名为空、
重启后幂等回读，交易/聚合器报价/聚合器提交记录为 1/0/0。CPMM→AMM v4
回归也得到 `simulated`；CPMM→PumpSwap 的池价移动使交易安全记为 `failed`，
未切到聚合器。

后续 `candidate-dd1f6235150a-6823e02a5c33` 还支持从选定 CPMM 或 AMM v4
目标池读取 Quote Mint。省略 Quote Mint 的隔离回归中，CPMM 目标代币增量
23,778,226 ≥ 最低到手 23,538,381；AMM v4 为 52,448 ≥ 51,923；
两者均为 `simulated`、签名为空，并通过重启幂等回读。可用以下命令复测
CPMM 新路线：

```bash
python3 scripts/verify_funded_sdk_preview.py \
  --binary /root/solana/trade_service/builds/candidate-dd1f6235150a-6823e02a5c33/pump-trade-service \
  --candidate-dir /root/solana/trade_service/candidate \
  --wallet-db /tmp/candidate-staging-data/wallets.sqlite \
  --wallet-id 2TE --cpmm-simulate-only --omit-quote
```

2026-11-06 的 candidate 加入 PumpSwap 单池 exact-in 买入报价，并让两跳预检共用严格
池快照与费率加载。公开池隔离 HTTP 回归确认报价、7 条双跳指令、未知钱包拒绝及零交易写入；
有资金钱包数据库副本的 unsigned 模拟返回 `simulated`，签名为空。缺少费率配置或存在
未知 cashback/非零转账税时拒绝报价，不会转向 Jupiter。此构建未安装到 18887。
报价到期时间现在不超过已验证路线的到期时间，RPC 读取耗尽路线 TTL 时直接拒绝报价。

最新只读池解析回归：`/v1/pools/resolve-candidates` 对明确 PumpSwap 池 ID、
省略 Quote Mint 的请求，从链上池读取并核验 Quote Mint；公开 WSOL 池返回
`chain_verified=true`，把目标 Mint 换成无关 Mint 返回 502。隔离 HTTP 全套通过，
`trades`、`aggregator_quotes`、`aggregator_submissions` 均为 0。该改动只改候选池解析，
不会签发非 WSOL 两跳执行许可，也未安装到 18887。

后续 candidate 又让 `/v1/routes/decide` 直接发现 Pump/PumpSwap SOL 买入的 Quote Mint。
公开 WSOL 池在省略 Quote Mint 时返回 `sdk_verified`；TTWO 非 WSOL 池返回短期
`observed_bridge` 和 `unresolved_retryable`。有资金隔离钱包副本的 TTWO 两跳 unsigned
模拟得到 `simulated`、签名为空，重启后精确重试只回读原结果；三张交易相关表分别为
1/0/0。池价在两次模拟之间下移时也曾返回 `failed` 且不更换执行后端。
最新 candidate 还把成功预检转为服务端 `sdk-decision:bridge:`；该决策只允许
原请求的 simulate-only unsigned 两跳模拟。接单和执行前重新核验预检与钱包；
隔离复测得到 `simulated`、签名为空、三张交易表 1/0/0，重启后精确重试
只回读原记录。Jupiter 不参与该订单。可用下方 HTTP 回归脚本和
`verify_funded_sdk_preview.py --omit-quote` 复测。
随后能力标记改为 `simulate_only_bridge_decision`，成功预检换取的决策附带
`route_evidence=unsigned_simulation_verified`，但 `jupiter_candidate_allowed=false`。
2026-11-06 隔离复跑时，一次 RPC 读取耗尽五秒观察有效期而返回 502；再次复跑
预检与决策通过，执行阶段因池价移动低于绑定最低到手而安全记为 `failed`，
签名为空、聚合器表仍为零。五秒观察 TTL 与慢 RPC 的冲突仍需优化。
验收脚本随后改为在同一份成功预检上完成回读、篡改拒绝、决策签发和交易模拟，
避免再为交易额外请求一次链上预检。使用未改动的 `candidate-5c34243afba1`
归档二进制复测得到 `simulated`、签名为空，三张交易表为 1/0/0；该脚本优化
不放宽服务端快照到期或最低到手校验。
下一 candidate 在成功的 unsigned 模拟接近观察到期时，重新从链上核验两腿身份并
签发新的短期观察 ID；池、Mint、Program、钱包或请求变化时拒绝刷新。
隔离有资金副本回归实际触发 `bridge_observation_refreshed=true`，后续池价低于
预检最低到手，交易安全记为 `failed`、签名为空，三张表 1/0/0；无资金 HTTP
路线回归及 138 项 Rust 测试通过。刷新提高慢 RPC 下的预检可用性，不放宽
执行阶段的当前池价校验。

```bash
python3 scripts/verify_funded_sdk_preview.py \
  --binary /root/solana/trade_service/candidate/target/release/pump-trade-service \
  --candidate-dir /root/solana/trade_service/candidate \
  --wallet-db /tmp/candidate-staging-data/wallets.sqlite \
  --wallet-id 2TE --omit-quote
```

## 最新 SDK 两跳路线证据

早期隔离 release `candidate-e191726a59b5` 已通过 123 项 Rust 测试、锁定 release 构建、
格式检查及下方正负向 HTTP 脚本。公开 TTWO/PumpSwap 目标池和 WSOL/TTWO Raydium CPMM
桥接池返回短期 `observed_bridge`；`/v1/routes/decide` 保持
`unresolved_retryable / bridge_verified_sdk_capability_unresolved`。观察 ID 和篡改请求
均不能授权交易，重启后旧决策失效，trade、aggregator quote、submission 表均为 0 行。

`POST /v1/routes/observations/sdk-preview` 重新读取两池并调用 Trade SDK 11.0.1
`PumpSwapQuoteRoute` 构建器。公开 fixture 构建 7 条指令，包含 Raydium CPMM 后接 PumpSwap；
桥接最低到手为 438 单位，目标最低到手随池状态变化。unsigned legacy 报文约 1,368
字节，超过 1,232 字节上限。此前报告的 SDK V1 约 1,395 字节使用了不适用的
`bincode`；当前源码已改用 V1 wire format 的 `wincode`。
隔离复测得到 1,387 字节，低于 V1 的 4,096 字节上限；大小会随具体路线变化。
当前 RPC 接受了 V1 模拟请求，
但一次性钱包无资金，模拟返回付款账户扣款失败。此结果只证明 RPC 模拟层接受 V1，
尚未证明协议成功执行、发送通道支持或可签发 SDK 决策。

公开池只读 RPC 偶发返回 `bridge_route_not_resolved`。脚本只对该可重试结果和目标池
RPC 暂时不可用最多重试三次；其余错误直接失败。脚本需要主机 Python 的
`cryptography` 包。该 release 未安装到 18887。

```bash
python3 scripts/verify_route_decision_http.py \
  --binary /root/solana/trade_service/builds/candidate-43411abf4944/pump-trade-service

python3 scripts/verify_route_decision_http.py \
  --binary /root/solana/trade_service/builds/candidate-43411abf4944/pump-trade-service \
  --positive-pumpswap-fixture --rpc-url-file .env
```

此前的隔离 release 依次验证了单池决策、WSOL/Quote 桥接池发现、短期观察快照和
零写入边界；其 `BUILD_INFO` 保存在各自的 `builds/candidate-*` 目录。当前脚本增加了
SDK 指令预检与 V1 模拟断言，应使用上述最新 release 运行。

## 有资金钱包隔离模拟

`scripts/verify_funded_sdk_preview.py` 将 staging 钱包 SQLite 以一致性备份复制到自动
删除的临时目录，使用隔离端口、临时交易数据库和 `TRADE_SERVICE_SIMULATE_ONLY=true`
启动指定 candidate 二进制。它先调用 `/v1/routes/decide` 与
`/v1/routes/observations/sdk-preview`；后者的 V1 报文使用默认签名占位符并设置
`sig_verify=false`。随后脚本把短期预览 ID 提交 `/v1/routes/decide`，用所得
`sdk_decision_id` 调用一次 `/v1/trades` 模拟入口，
要求目标增量达标、交易状态为 `simulated`、签名为空，且聚合器 quote/submission
表为 0。数据库副本和进程在退出时清理。

```bash
python3 scripts/verify_funded_sdk_preview.py \
  --binary /root/solana/trade_service/builds/candidate-000ec74b9abb/pump-trade-service \
  --candidate-dir /root/solana/trade_service/candidate \
  --wallet-db /tmp/candidate-staging-data/wallets.sqlite \
  --wallet-id 2TE
```

2026-11-05 的公开 TTWO fixture 验证得到：目标代币增量 10,783,332 单位，最低到手
10,675,498 单位，付款账户扣款 4,078,400 lamports，消耗 171,693 CU；三张表均为 0。
该结果只代表此次钱包、金额、池状态和 RPC 的 simulateTransaction。
最终 release 复测时目标增量 10,704,474 单位，最低到手 10,597,351 单位，扣款
4,078,400 lamports，消耗 171,623 CU；同次模拟的账户余额和 token balance 均被核对。
`candidate-5ec7df2af568` 的 `wincode` 复测得到 V1 wire 1,387 字节，目标增量
10,764,545 ≥ 最低到手 10,656,899，扣款 4,078,400 lamports，消耗 173,037 CU；
三张交易相关表仍为 0。无资金正负向与重启 HTTP 检查也通过。
候选源码的离线 mock RPC 测试还核对 Trade SDK 与 Solana RPC 客户端的 V1 发送编码；
未向真实发送端点发请求，不能据此开放两跳交易。
最新隔离构建再次通过有资金模拟：V1 wire 1,387 字节，目标增量 10,588,094 ≥
最低到手 10,482,213，扣款 4,078,400 lamports，消耗 176,218 CU，三张表均为 0。
新源码只在 unsigned 模拟成功且目标增量达标时签发短期 `observe:sdk-preview:` ID。
当时的 `verify_funded_sdk_preview.py` 检查同一钱包和请求的只读回读、篡改拒绝、把预览 ID
当作交易决策使用时拒绝，以及重启后回读拒绝；该旧切片不会创建交易或提交记录。
`candidate-dba734bbe4ed` 隔离复测：目标增量 10,666,815 ≥ 最低到手 10,560,202，
V1 wire 1,387 字节，179,176 CU；预览 ID 边界均通过，三张交易表仍为 0。

最新隔离切片允许把成功的 `observe:sdk-preview:` ID 放在原请求的 `sdk_preview_id`
字段，向 `POST /v1/trades` 发起**显式 simulate-only** PumpSwap SOL 买入。
`simulate=true` 必填，且必须启用 durable 幂等存储；原请求的其它字段及幂等键
必须完全一致。服务端在占位前
校验短期预览，随后重新报价、构建并执行 unsigned V1 模拟，核对先前绑定的
目标最低到手。没有签名或广播路径。完全相同的幂等重试在 ID 过期及服务重启
后只回读原结果；过期 ID 不得创建新交易。
`candidate-6bd548ca48d7` 的隔离验收通过：126 项 Rust 测试；有资金副本
生成 1 条 `simulated` 交易、签名为空、quote/submission 为 0；无资金路由
HTTP 回归仍为三张表 0 行。此次有资金预览 V1 wire 为 1,387 字节，目标增量
10,615,712 ≥ 最低到手 10,507,688。运行中的 18887 服务未更新。
`candidate-90fe7aef60e1` 又在 `/v1/trades` 占位前拒绝客户端显式选择 Jupiter：
没有服务端 `sdk_unsupported` 决策时返回 409。无资金 HTTP 回归覆盖非 WSOL
桥接路线，trade、aggregator quote、submission 表均为 0；126 项 Rust 测试通过。
这项入口限制不影响 `/v1/aggregators/quotes/compare` 的只读 Shadow。
后续构建在已核验的 CPMM→PumpSwap `/v1/routes/decide` 响应中返回只读
`sdk_capability`：SDK 包具有 `PumpSwapQuoteRoute`，服务仅有模拟适配器，
`sdk_unsupported_proven` 与 `jupiter_candidate_allowed` 均为 `false`。HTTP 回归
核对字段值；决策状态仍为 `unresolved_retryable`。
隔离 release `candidate-204084ea49d5` 的 127 项 Rust 测试和无资金 HTTP
路由回归通过；显式 Jupiter 交易仍返回 409，三张交易相关表仍为 0。
隔离 release `candidate-000ec74b9abb` 的 128 项 Rust 测试及无资金 HTTP
回归通过。`/v1/capabilities` 现区分 SDK 包路线家族与服务端 SOL 路线适配器；
HTTP 检查确认 PumpSwap 仅有 CPMM→PumpSwap 模拟适配，Jupiter 交易仍要求
服务端 `sdk_unsupported` 决策。
隔离 release `candidate-e4fd832c2a5e` 的 SDK 决策 HTTP 回归还核对文件钱包模式的
有效 PumpSwap 决策模拟：一次性钱包无资金，交易记为 `failed`，返回签名为空；
聚合器提交表仍为 0。SDK 高层模拟会在本地构建并签署交易供 `simulateTransaction`
使用，服务端现丢弃该本地签名，不把它当作链上提交凭据。该请求未走发送路径。
当前 Helius RPC 对公开 fixture 的无池 ID Mint 枚举超时，因此该分支保持
`unresolved_retryable`；明确池 ID 的同一目标仍签发短期 SDK 决策。

```bash
python3 scripts/verify_route_decision_http.py \
  --binary /root/solana/trade_service/builds/candidate-e4fd832c2a5e/pump-trade-service \
  --positive-pumpswap-fixture --rpc-url-file .env --simulate-valid-decision
```

## 固定边界

- 只使用 candidate 的已核验 release binary；隔离测试可从 `builds/` 复制运行，
  不覆盖当前 `18887` 正在使用的 release 文件。
- 使用独立 `trades.sqlite`、`wallets.sqlite`、`conversions.sqlite` 副本。
- 使用非生产端口，例如 `127.0.0.1:18887`。
- `TRADE_SERVICE_SIMULATE_ONLY=true`、`ENABLE_SIMULATE=true`、
  `CONVERSION_LIVE_ENABLED=false`。
- 不启动 `current`，不改 `127.0.0.1:18788`，不连接 bot live unit，不签名或广播交易。

## 启动前检查

```bash
test "$(readlink -f /root/solana/trade_service/current)" = \
  /root/solana/trade_service/releases/sdk-6.0.2-legacy
test "$(systemctl is-active trade-service-candidate.service 2>/dev/null || true)" != active

# If the Bot workspace is present, verify its read-only boundary without starting it.
python3 /root/solana/bot_trading/scripts/verify_simulate_only_config.py
grep -Fq 'ExecStartPre=/usr/bin/python3 /root/solana/bot_trading/scripts/verify_simulate_only_config.py' \
  /root/solana/bot_trading/bot-trading.service
```

复制数据库到 staging 目录后，使用环境变量覆盖地址：

```bash
export CANDIDATE_STAGING_URL=http://127.0.0.1:18887
export TRADE_SERVICE_LOCAL_API_TOKEN='staging-only-token'
export CANDIDATE_STAGING_DB=/srv/candidate-staging/trades.sqlite
```

服务进程必须使用独立 `EnvironmentFile`，其中 `TRADE_HTTP_ADDR=127.0.0.1:18887`，
数据库 URL 指向副本，且不得引用生产 wallet passphrase 或生产数据库。推荐安装
`trade-service-candidate-staging.service`，由 systemd 保持 candidate 运行；再安装
`jupiter-shadow-staging.service`。sampler 会等待 candidate 的 `/ready` 最多 300 秒，
candidate 重启期间记录等待/失败状态，恢复后继续采样。
staging unit 通过 `ExecStart=/usr/bin/env ...` 覆盖基础 `.env` 的 `18787` 和生产数据库
地址；只在 unit 文件中写 `Environment=` 不能覆盖 `EnvironmentFile` 的同名键。

## 验收顺序

1. 启动 candidate 并确认日志只显示 `18887`。
2. 执行 `scripts/staging_http_smoke.sh`。它检查 health/ready/capabilities，执行一次
   DFlow-only compare，确认 `shadow_only=true`、`unavailable` 和 Shadow 持久化。
3. 停止进程，再启动同一 candidate 和同一数据库副本。
4. 执行：

   ```bash
   CANDIDATE_STAGING_DB=/srv/candidate-staging/trades.sqlite \
     scripts/verify_staging_persistence.py
   ```

5. 确认日志、SQLite 和端口仍属于 staging；重复 smoke 后才开始 Jupiter read-only Shadow。
   Shadow 必须使用独立运行日志，日志首行包含 `shadow_starting`，末行包含
   `shadow_complete=true samples=N`；重启或旧日志不能拼接为同一 24 小时证据。
   完成后使用同一份日志和同一份 SQLite 执行：

   ```bash
   scripts/verify_jupiter_shadow.py \
     --log /tmp/jupiter-shadow-runs/run-<run-id>.log \
     --db /tmp/candidate-staging-data/trades.sqlite
   ```

   该检查要求至少 24 小时、至多每 60 秒采样一次、日志时间戳严格递增，并逐条核对日志
   与数据库中 Jupiter metric 的状态和 31 秒内写入时间；报价成功率至少 99%、p95 不超过
   3000 ms、p99 不超过 10000 ms，且不能有缺失或负数延迟，
   所有 `quoted` 指标还必须有完整的预期输出、最小输出和合法滑点，最小输出须达到
   请求滑点下界；字段缺失或不合格会直接拒绝验收。任意格式损坏的 HTTP 样本或重复完成记录也会拒绝，
   且该时间窗没有持久化 quote/submission 或未完成 reservation。阈值可以通过脚本
   参数调整，但必须在运行前记录；不能在看到结果后放宽。脚本以只读模式打开 SQLite，
   不会拼接旧运行或删除数据。

   新版 Shadow summary 同时给出 `min_output_checked_count`、`min_output_missing_count`、
   `min_output_coverage_rate` 和 `min_output_pass_rate`。两个比率都以 `quoted_count` 为
   分母；字段缺失的报价计入未通过，不能因为只检查了少数报价就显示 100% 通过。
   该窗口运行时 18887 使用旧二进制；其验收以归档日志和 SQLite 只读核对为准。
   窗口结束后的新二进制可通过 HTTP 查看新增字段，但不能把新响应并入旧窗口。

   运行途中可用独立的只读检查提前发现重复指标、缺样、状态漂移或意外的 quote/submission：

   ```bash
   python3 scripts/audit_jupiter_shadow_progress.py \
     --log /tmp/jupiter-shadow-runs/run-20261105T004653Z-425291.log \
     --db /tmp/candidate-staging-data/trades.sqlite
   ```

   途中检查也会逐条验证已记录报价的最小输出，并报告检查覆盖数和通过数。
   其中 `passed_so_far=true` 只说明已产生样本的一致性；必须等完整窗口结束，再运行
   `verify_jupiter_shadow.py` 判定 24 小时验收。

   离线回归测试：

   ```bash
   PYTHONPATH=scripts python3 -m unittest -v \
     scripts/test_verify_jupiter_shadow.py \
     scripts/test_audit_jupiter_shadow_progress.py
   ```

   2026-11-05 的第一份 `run-20261104T234942Z-365518.log` 被遗留手工 sampler
   并行写入的 metrics 污染，已废弃。有效窗口从
   `/tmp/jupiter-shadow-runs/run-20261105T004653Z-425291.log` 重新计时；不得将旧日志
   或旧 metrics 拼入新窗口。该窗口已于北京时间 2026-11-06 08:47:47 正常结束，
   `verify_jupiter_shadow.py` 返回 `passed=true`：86,454 秒、1,439 条 `quoted`、
   p95 36 ms、p99 48 ms，最小输出覆盖率及通过率均为 100%，窗口内
   quote/submission 为 0。持久证据在
   `/root/solana/trade_service/evidence/jupiter-shadow-20261105T004653Z/`；
   归档日志与 SQLite 备份再次通过只读验收。sampler 已正常退出并关闭开机自启。
   窗口期间不要覆盖 `target/release/pump-trade-service`：staging unit 设置了自动重启，
   若旧进程意外退出，会加载磁盘上的新二进制，造成同一日志混用两个版本。
   新代码可通过 `scripts/verify_candidate_gates.sh` 在独立 target 构建 release；
   18887 的部署已在窗口结束后进行，保留旧二进制回滚副本。
   Bot 不参与此 smoke；若 Bot workspace 存在，启动前置检查必须通过，但不得启动 Bot 或
   使用 `127.0.0.1:18788` 作为 candidate staging 地址。

## 24 小时 Shadow 进入条件

- HTTP smoke、重启回读和 DFlow unavailable 均通过。
- 没有 `reserved/submitting` 残留、未知订单、钱包公钥漂移或重复 reservation。
- 仅允许指定钱包、指定协议和 dry-run 策略。
- 记录 quote、simulation、持久化、回查和端到端 p50/p95/p99 延迟。
- 任一失败都停留在 simulate/observe，不进入 canary 或 live。
