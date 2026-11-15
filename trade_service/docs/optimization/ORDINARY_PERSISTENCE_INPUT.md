# TS-02-C1 普通持久化内部输入

日期：2026-11-13。状态：新增 7 项离线测试，全套 294 → 301 通过，无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、修改生产配置/数据库/钱包或发送交易。

## 已实施

`application/trade_persistence_input.rs`：

- `StoredRequestV1`：原 hash 和原 `request_json` 的不透明拥有型字符串；私有字段、无 serde、无 SDK 类型，不重序列化旧 JSON。
- `ValidatedTradeInput`：绑定解析后的 wallet_id、原 idempotency_key、原 aggregator 分支位和 V1 快照。数据库 reserve 只接受此类型。
- `RejectedTradeInput`：绑定 wallet_id 和被拒绝 request_id，生成原 `__rejected__:<request_id>` storage key。不能传给 reserve_input。
- 输入仅是存储载荷，不是链上已验证路由、资金授权或可执行 command。

`adapters/persistence/trade_snapshot.rs`：

- 集中 DTO/NormalizedTrade → 内部输入、旧 hash、raw rejected hash、原 JSON 编码、已存 JSON 的身份/输入分支核对。
- 原 `TradePersistence::reserve/mirror` 签名保持为兼容桥，实现在 codec 模块；`persistence::request_hash` 保留重导出。
- codec 保持原 memory/非 durable 跳过规则、先检查存储存在再构造快照的顺序，以及原错误文本。

`persistence.rs`：

- 数据库核心改用 `reserve_input`、`mirror_validated`、`mirror_rejected` 与内部快照写入。
- 生产核心不再接收/解析 TradeRequest、NormalizedTrade 或 SDK Pubkey；上述名字在本文件仅为 `cfg(test)` 测试使用。旧存储 JSON 的解释由 codec 完成。
- 原 SQLite insert/update/恢复、shadow writer、schema、record 格式和 mode 行为不变。新输入在实际存储前校验其 wallet/request 绑定，防止未来内部调用将已准备快照错配到另一记录；兼容桥按原 record 构造，既有外部请求不新增限制。

普通调用方仍经原桥传 DTO；后续接受编排尚未迁到 Application。数据库记录类型仍来自旧 models 路径，不能宣称全部持久化已脱离 API/所有跨层依赖。

## 迁移前字节证据

先在旧 reserve/mirror 实现上执行真实临时 SQLite 写入：18 个正常归一化样本、19 个拒绝样本；读取原 idempotency_key、request_hash、request_json 字符串形成联合指纹，再提取实现并固定断言。

```text
d322ae4c28b37aae3bd3db33c136fec523e75792a65d997d25e1da8f015e8653
```

各样本另核对 JSON 字符串与旧 DTO 序列化完全一致、已接受 key 字面值和 rejected 前缀。测试使用确定性合成 key/request_id，不包含真实钱包或订单。

此前 18 组 wire/normalization/hash 指纹 `cf2a091b...`、API 合同和旧 JSON 文件重开/恢复测试均保留；没有更新期待值来接受迁移后差异。

## 保留规则

| 项目 | 本批行为 |
| --- | --- |
| legacy hash | 原字段/名字/JSON 编码/float bits、三态 simulate 和旧 settlement 格式不变；没有把 aggregator 位加进旧 hash |
| 分支防碰撞 | 原 hash 相同后继续验证已存 JSON 身份和 SOL aggregator 分支；损坏/不可验证仍 fail closed |
| alias/元数据重放 | 旧 key/hash/分支相同仍 duplicate；原 JSON 不因 source 或金额 alias 不同而重写 |
| rejected | raw request hash，不误用 normalized hash；独立 storage key，不占原用户 trade key |
| memory | reserve/mirror 保留原跳过存储行为，不因新增快照绑定检查改变 memory 返回 |
| 非 durable reserve | 原直接 Created，不执行 durable 保留 |
| shadow | 仍把记录/原 key/hash/JSON 放入原有队列，错误/health 行为和异步插入保持 |
| durable | 原 insert-or-ignore、原冲突/重复/错误、原重启 unknown 恢复；无 schema 或历史数据迁移 |

原 hash 的遗漏并非全部解决；本批延续既有分支补验，不把 V1 快照类型当作完整业务等价性或发送前签名 journal。

## 新增测试

`persistence/snapshot_contract_tests.rs` 共 7 项：

1. 37 组真实入库字节联合指纹。
2. 已校验快照不能 reserve/mirror 到另一钱包，拒绝前数据库 0 行。
3. 拒绝快照必须同时匹配 wallet_id/request_id，拒绝前数据库 0 行。
4. rejected 与合法交易使用同一用户 key 时仍独立保存，合法交易可保留/重放。
5. 金额 alias/source 变化的 typed 重放不重写原 JSON。
6. memory 保留跳过规则；不可用 shadow 存储先报原存储错误，而非新的绑定错误。
7. 真实 shadow writer 消费两种 typed 快照，保存正确 key 与 JSON，有界等待完成。

首次全套出现的唯一失败是新 shadow 测试把 request_id 排序与 storage-key 排序的期待混用；修正的是新测试查询，不是生产写入或冻结指纹。最终全套通过。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet persistence::
cargo test --locked --offline --quiet
git diff --check
```

结果：301 passed、0 failed、无编译警告。现有 48 项 HTTP 合同、状态分类和 SDK 参数基线保持。Cargo.toml、Cargo.lock、现有 release 指纹与前批一致，vendor/现有迁移文件 diff 为空。本批没有新数据库迁移。

## 下一批 TS-02-C2

2026-11-13 续作：V1 record 内部归属、静态 ordinary journal port 与实际写入/查询接入已完成，新增 7 项、全套 308 项通过，见 `ORDINARY_RECORD_AND_JOURNAL.md`。HTTP 接受编排、wire validator 和签名前账本仍待后续，以下保留本批要求。

1. 明确旧 TradeRecord 的内部记录/读模型归属，保留旧导入路径和固定 serde/数据库字段；不要把该 legacy snapshot 冒充已经完成的领域交易聚合。
2. 以内部记录和 typed inputs 建立普通 journal/存储 port，应用调用不再依赖 API DTO、NormalizedTrade 或 SDK 类型；codec 只在边界处理 V1。
3. 保持原错误/状态/副作用顺序，并用本批 37 组字节指纹与真实 HTTP 成功/并发/冲突/批量/503 回归保护后续迁移。
4. 普通接受仍依赖 AppState、HTTP 类型与 validator；逐步迁移其编排，不通过仅换目录宣称完成解耦。
5. 所有普通资金路径的发送前签名账本、完整 VerifiedRoute、SDK provenance/升级、真实节点验收和部署仍是独立门槛。
