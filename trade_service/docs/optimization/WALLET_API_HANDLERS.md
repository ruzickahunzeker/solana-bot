# NP-02 / TS-08-E 钱包 API Handler 与路由归位

后续状态：TS-08-F 已将两个余额 Handler 的业务接入 Application/metadata/runtime port，G 又收口六个管理 Handler 的 Application/port，分别见 `WALLET_BALANCE_APPLICATION_BOUNDARIES.md`、`WALLET_MANAGEMENT_APPLICATION_BOUNDARIES.md`。下文直接解析 SDK/获取 read-access/钱包管理接具体仓储等限制仅为 E 的历史状态；vanity 及完整资源/生命周期边界仍待续。

日期：2026-11-14。范围仅源码和离线验证，未部署。目标是完成既有 HTTP 入口归位，不改变钱包生命周期、密文格式、所有权、SDK 初始化或资金行为。

## 变更

- `api/handlers/wallets.rs`：原列表、管理列表、创建/导入、改名、删除、导出六个 Handler，以及原响应映射。
- `api/handlers/wallet_balances.rs`：原 SOL 批量余额及 Quote 余额两个 Handler。
- `api/handlers/vanity.rs`：原创建/查询/取消靓号任务三个 Handler。
- `api/routes.rs`：三个对应路由组；保持原 URL、方法、方法组合、总路由合并次序、HEAD 和静态路径优先级。
- `api/dto/wallets.rs`：原十个请求/响应 DTO 和 OwnerQuery，原字段、顺序、Option、Serde 派生保持。`models` 兼容重导出；共享 `WalletInfo`、`VanityMatchSide` 暂不扩大本次移动范围。
- `http.rs` 由 412 行收为 17 行路由组合；原鉴权包装移除，Handler 直接调用同一 auth 函数，参数及顺序保持。没有旧 Handler 转发或重复注册。

本次没有新增 URL、配置、策略、API 字段或外部依赖。创建/导入的仓储先写、runtime add 及新建时 disable 补偿原序保持；导出成功的 `Cache-Control: no-store`、旧错误映射保持。

## 验证顺序

七项新合同先在原 `http.rs` 位置通过，搬迁后原七项再次通过，再增加三项独立路由组验收。样本只含公开地址、非密钥占位字符串与临时 SQLite 公共元数据；不初始化真实 SDK/RPC、不生成钱包、不解密密文、不运行靓号 worker、不签名或广播。

覆盖范围：

1. 十个 DTO 中重点字段/整数/Option/大小写规则，以及原列表、改名、默认钱包删除保护。
2. 原 owner/enabled 检查先于 Quote Mint 解析，再先于已加载 runtime 查询；system/global 可见规则保持。
3. 批量余额的文件模式错误和坏公共地址早拒绝；尚未使用真实 RPC 或把 fake 成功冒充余额成功。
4. 靓号原终态任务 GET/DELETE、所有者错配的不同错误、坏 pattern 不新建任务/worker。
5. 提取器在 Handler 鉴权之前的拒绝、HEAD、方法不支持、静态 manage 路径、独立组隔离。
6. legacy/separate 管理凭据与交易/余额凭据分离，loopback 规则，禁播策略不改变只读钱包入口。
7. 无 passphrase 的导出错误、非法非密钥导入早拒绝；未对成功导出/创建/SDK add/真实 worker 成功作出验收声明。

相对 C2b 新增十项；原位置七项通过，移动后七项通过，再补三个组合同后专项十项全部通过。最终验证：

```bash
cargo test --locked --offline --quiet wallet_boundary_tests  # 10 passed
cargo test --locked --offline --quiet                        # 939 passed
cargo test --locked --offline --quiet -- --test-threads=1     # 939 passed
cargo test --locked --offline --quiet http_contract_tests    # 183 passed
cargo check --locked --offline --bin pump-trade-service      # passed
cargo fmt --check                                           # passed
git diff --check                                            # passed
```

只余原 models/persistence 两组兼容导出警告，无新增抑制；旧 V1 hash/JSON、2,016 组 SDK 参数、98 组状态映射、576 组 DAMM 消息黄金合同随全套通过，未重写期望。不以专项或 fake 测试替代真实 SDK/节点验收。

## 保持的限制

- 本阶段完成 Handler/DTO 归位，不等于钱包 Application/Port 分层完成。钱包管理仍接原具体 repository/registry，余额 Handler 仍有原 Pubkey 解析与 read-access 获取；下一阶段继续收口，不新建第二套钱包权威或把 signer 暴露到 API。
- 现有部分失败行为已冻结：DB 钱包可见不代表 runtime 已加载；删除先 disable，再 remove，不存在 runtime 时返回 500，但 DB 已禁用。此次不悄悄改变该生命周期；若修复需要单独成功/失败/补偿合同。
- 余额成功尚需可注入的只读 runtime/metadata port，不能为测试初始化带密钥的 SDK 或构造假的默认交易客户端。
- 普通订单 GET 现有合同只读存储；本阶段不会把它变为隐式 RPC 对账或改变等待/错误预算。NP-01 普通原签名只读对账仍在队列，必须显式界定触发边界、身份/原字节绑定、最终性/CAS 和错误合同，不得借 GET 超时重发交易。
- C2b 仅覆盖 Durable Live 普通 SOL 聚合；其他模式/backend 的账本、最终消息授权、SDK 来源/升级演练及真实验收仍未完成。PumpAPI 交易对接开发保持暂停。

## 受保护文件

本阶段前后 fingerprint 核对范围包括 wallet repository/registry/ops、RPC wallet reader、executor、PumpAPI 实现、manifest/lock 与旧 release。它们本阶段没有改动；具体 SDK/钱包行为不能因文件移动被声称已修复。

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

必要 API 接线有意改变：`http.rs` 的 SHA256 从 `b839b4dacb437c4e6faaf9c5725ccecfd584b8c7e7627b6647b2af5acb352bc7` 变为 `dd1a639b5d1ebd698b15df2921907d15a9c2aabf3d57d967104636eba9356166`；`models.rs` 从 `f14a564e60c39e98e4fb8644ea49f3ada885ffd50e9b6d4b9ea0d0c83cf4cf5d` 变为 `14fc8955f701c709876ea15e1e740faf85c9fafbb4507bdf9df23d8da93ff2e2`。这不是业务逻辑或密文格式变化。

持续目标 active；修改计划、阶段总结和汇报不暂停。无 release 构建、部署/重启、生产配置/钱包/数据库操作、提交或推送。
