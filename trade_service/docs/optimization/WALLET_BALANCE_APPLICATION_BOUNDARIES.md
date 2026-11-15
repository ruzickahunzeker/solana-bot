# NP-02 / TS-08-F 只读钱包余额 Application 边界

后续状态：TS-08-G 已收口六个钱包管理 Handler 的 Application/port，见 `WALLET_MANAGEMENT_APPLICATION_BOUNDARIES.md`。下文“钱包管理分层待续”为 F 的历史剩余项；vanity、原子生命周期与生产验收等仍未完成。

日期：2026-11-14。源码与离线验收阶段，未部署。范围为两个已有余额 GET，不扩大钱包管理、交易、兑换或 PumpAPI 能力；本阶段不是整个钱包 Application 完成。

## 实际边界

- `application/wallet_balances.rs` 持有查询编排、可见性政策、项目整数结果及错误种类，不依赖 Axum、SDK、AppState、具体钱包仓储或 RPC。
- `application/ports/wallet_balances.rs` 分开公开元数据读取与运行余额 reader 获取；后者只返回原 `WalletReadPort`，没有 signer、原始交易/客户端、预留/提交/广播方法。
- `adapters/persistence/wallet_metadata.rs` 用原仓储 Arc 的 `get`/`list` 投影公开字段，不新建连接、业务库、缓存、权威或 SQL；没有读取/解密密文方法。
- `adapters/sdk/wallet_balances.rs` 用原 registry Arc 选择原 `read_access`/`default_read_access`，复用原 RPC wallet reader，不创建 SDK/RPC 客户端，不改变节点、回退、超时或金额算法。
- 复用原 `SdkAddressParser` 的纯语法转换；地址返回转为项目 Address，解析不修剪、不增加 Quote alias 或链上校验。
- `api/handlers/wallet_balances.rs` 仅保留原提取器/鉴权、调用查询与 HTTP 映射；DTO 只映射项目只读结果，不获取 runtime 或解析 SDK Pubkey。
- bootstrap 注入同一组 repository/registry Arcs 的真实服务。测试若替换仓储，也重建对应服务；不是把生产原 authority 换成新字典。

原 `models::WalletBalanceItem` 已无生产消费者，按现有兼容桥模式保留为 cfg(test) 重导出；正式 DTO 定义仍在 API，序列化字段及旧测试名称不变。没有为压掉警告在 API 回引旧兼容层或添加 allow；这不是对外交易接口变更。

共享五字段元数据归 `domain/wallet.rs::WalletMetadata`。原 `WsolWalletMetadata` 仅改为兼容别名，不改既有 WSOL loader、owner、异常类型、签名/提交流程或持久化格式；该必要共享类型变更须由全套 WSOL 合同回归，而不是宣称 WSOL 源码完全未改。

## V1 行为保持

| 分支 | 保持的顺序与合同 |
| --- | --- |
| Quote | 原仓储 get → enabled/owner 或 system/global → Mint 语法 → 原 loaded reader → Quote 余额 |
| Quote 失败 | get 任意错误为原 404/原 context；owner 为 403；Mint 为固定 400；未加载为 503；RPC 为 502 且按原规则 URL 脱敏 |
| SOL 批量 | list → 过滤 enabled 和同 owner/system/global → 按原顺序解析可见公钥 → 原 Default reader → RPC → 构造结果后采时 |
| SOL 批量失败 | list 为原 503/原错误；公钥为原 500/原解析文案；RPC 错误仍为 200 并给每个可见钱包 null balance/原 error |
| 金额与身份 | u64 原始整数，u128 原时间戳，字面 wallet_id/owner、原公钥字符串、原元数据顺序；无默认归一化或浮点换算 |
| 路由/鉴权 | 两个已有路径、HEAD、Query 提取顺序、loopback、交易凭据与管理凭据区分全部保持 |

get 使用 `wallet not found` context，list 不同；连接池关闭时不能假定两者文案相同。专项初次暴露的是该测试预期混淆，已改为分别核对两个原仓储结果，生产查询没有统一覆盖错误。

## 离线证据

新增测试包含八项 Application（纯假 metadata/parser/reader/clock）、两项真实临时仓储 projection/error、一项空 registry Arc 复用，以及四项真实进程内 HTTP/临时元数据库 + fake readonly runtime。样本只有公开地址、非密钥字符串和固定整数；没有 .env、真实 SDK 初始化、供应商网络、密文解密、私钥创建/导出、签名或广播。

- 精确 owner/system-global/enabled、literal ID、首个坏地址短路、隐藏坏元数据不解析、调用/时钟顺序。
- Quote u64::MAX 原始整数；批量排序/过滤/原 pubkey 与 null/error、HTTP 完整 JSON。
- 两类仓储错误上下文、未加载、Quote 脱敏与批量原 per-row error 差异，不把只读错误误当作资金结果。
- 单次 fake reader Future 取消时的 Drop，不出现自动 retry 或后台查询；不把这个 fake 合同当成真实 RPC/SDK worker 可取消的证据。
- Arc 身份、临时库实时元数据变化、无 audit/decrypt 访问；旧 WSOL 别名继续构造同一五字段类型。

相对 TS-08-E 新增十五项（8 Application + 2 metadata + 1 runtime + 4 HTTP）。production check 初次发现新无消费者兼容导出，已收紧为 cfg(test)，再次 check 通过且只余原 models/persistence 两组警告，无新增抑制。收紧后最终源码的验证如下，不以收紧前的回归冒充最终记录：

```bash
cargo test --locked --offline --quiet wallet_balance                        # 14 passed
cargo test --locked --offline --quiet adapters::persistence::wallet_metadata # 2 passed
cargo test --locked --offline --quiet                                       # 954 passed
cargo test --locked --offline --quiet -- --test-threads=1                    # 954 passed
cargo test --locked --offline --quiet http_contract_tests                   # 187 passed
cargo check --locked --offline --bin pump-trade-service                     # passed
cargo fmt --check                                                          # passed
git diff --check                                                           # passed
```

旧 V1 hash/JSON、2,016 组 SDK 参数、98 组状态映射、576 组 DAMM 消息及 WSOL 原有合同随完整并发/串行通过；没有重写黄金期望。源码生产路径已接通，不是未使用的预留接口；fake 成功不代表真实节点或钱包验收。

## 指纹与实施范围

仓储/registry/ops、原 RPC wallet reader、executor、PumpAPI 实现、Cargo.toml/lock 与旧 release 均核对为本阶段前指纹：

```text
0609cd75ff6f887fe0c42d1114703e337db5c8f9eb3f58f7c97eb6aaf8aede74 src/wallet_repository.rs
162ab6d5c1926bc8378d600a7c5a7f186b8f60c408a7703fa0d768ad65c5f0df src/wallets.rs
c7b0904e48af8e12297dae745fe8625a06e35cb53337afc9adc33956d38e75f0 src/wallet_ops.rs
d721589eac8295e32387abc5c1aa8ed146d199c0d9206677b9c5c04b94e2a666 src/adapters/rpc/wallet_reads.rs
bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec src/executor.rs
acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b src/pumpapi_client.rs
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

main/AppState、余额 Handler/DTO、models 测试兼容桥、domain 与 WSOL 元数据别名有意改变，不冒称全部旧文件未改。当前主接线指纹：

```text
1503696f3a6203b386faf81fcbbe76204e4b3635806b93354e0fd8462f557b40 src/main.rs
9fad919a4ab766f14b7f0a337d2276fdd67f2fa832765f06d0c858ad85df5533 src/api/handlers/wallet_balances.rs
a7a585884662148b52d0fc6411ea1ecbe3fa92cc4a65c2e504f636dc80609c1e src/models.rs
6de46a011fbcecfc267e637f874e0414005f87baa64b6d3c18b30ed44a4beb5c src/application/ports/wsol_conversion.rs
```

## 没有解决的风险

1. 仅收口余额查询；钱包创建/导入/改名/禁用/导出和 vanity 生命周期 Application/Port，以及 AppState/bootstrap 其他资源仍待续。
2. 原批量结果用 zip，异常少/多结果维持旧截断行为；空可见列表仍走原 Default 查询，没有新增长度证明或空查询优化。默认钱包应存在的原启动/runtime 假设未改变。
3. 原批量 per-row RPC error 不新增脱敏规则，不能将兼容回归解释为已完成全部凭据输出审计。改变这项策略需要独立安全合同。
4. Quote 原整体 deadline、统一 RPC 并发/配额、真实节点与余额正确性仍是后续验收，本阶段不以假数据宣称节点正常或查询更快。
5. 普通交易最终授权、其他模式/backend 签名账本、原签名对账、SDK 来源/隔离升级演练及生产验收仍未完成。普通订单 GET 不新增隐式 RPC；PumpAPI 交易对接保持暂停。

不升级 SDK/依赖、不构建 release、不部署重启、不访问生产配置/钱包/数据库、不提交推送、不重发订单。持续目标 active，修改计划或汇报不暂停。
