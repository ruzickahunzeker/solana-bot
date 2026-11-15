# NP-02 / TS-08-G 钱包管理 Application 边界

后续状态：H 已收口靓号任务控制的 Application/worker 输入边界，见 `VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`。下文 vanity 分层待续仅为 G 的历史剩余项；实际生成/SDK 加载、原子生命周期和生产验收仍未完成。

日期：2026-11-14。范围为原六个钱包管理 Handler 的源码与离线验收，已通过，未部署。不是全部钱包生命周期、安全审计或资金路径完成。

## 实际接线

- `application/wallet_management.rs` 接管 loaded/managed 列表、创建/导入分支、运行加载、原失败补偿、改名、删除顺序及显式导出；只收项目输入、公开结果与错误种类，不依赖 HTTP、AppState、具体仓储、WalletConfig、SDK 或 RPC。
- `application/ports/wallet_management.rs` 定义最少的原仓储和运行操作；仓储的关联 Material 只作为 owned 值传给 install，核心没有 Clone/Debug/Serialize 或读取内容的要求。
- `adapters/sdk/wallet_management.rs` 复用原 repository/registry Arcs。创建/导入仍调用原仓储；唯一私有 `WalletMaterial` 包住返回字符串，只有安装适配器拆开交给原 `WalletConfig`/`wallets.add`。不创建第二个仓储/客户端或新增密钥权威。
- `WalletMaterial` 无 Clone/Serde，Debug 固定脱敏；Application 输入及显式导出结果不派生 Debug。原 HTTP DTO 和 Config 的整体日志/内存安全不由此自动获得审计证明。
- `domain/wallet.rs` 持有原 WalletInfo/ManagedWallet 定义；models 和 wallet_repository 原名称兼容重导出，字段/顺序/派生保持。原钱包元数据持久化与只读余额服务继续使用同一类型，不引入 SDK 字段。
- 六个 Handler 只保留原提取器、鉴权、调用、状态/DTO 映射，以及显式导出成功的 no-store；不再构造 WalletConfig 或直接操作仓储/registry。
- bootstrap 初始化一次真实服务，两个 port 由同一适配器的原 Arcs 提供。测试替换仓储时同步重建该服务，不用旧 File-mode source 冒充新临时库。

## 保持的行为

| 操作 | 原顺序与结果 |
| --- | --- |
| loaded 列表 | 原 default_wallet_id 与实际已加载 infos；不以数据库全部行代替运行钱包 |
| managed 列表 | 原仓储顺序/全部行，含 disabled；created=false；失败仍 503 |
| 创建 | private_key 缺失/null 才创建；Some 空串/空格仍走导入，由原仓储解析拒绝，不偷偷生成新钱包；新建 created=true |
| 导入 | 原 owner/label/密钥字符串不在 Application 修剪；created 标记保留仓储对新建/恢复/已存在的原规则 |
| 加载 | 仅 enabled 才 install；调用原 add，不新增交易/重发逻辑；失败时仅 created=true 尝试一次 disable，忽略该补偿错误且保留原加载错误/502；真实 SDK 初始化另需验收 |
| 改名 | 只修剪 label，不改字面 wallet_id；失败仍 400 |
| 删除 | 先原仓储 disable，后同步 runtime remove；前者错误 400，后者错误 500；不悄悄恢复已禁用行 |
| 导出 | 仅显式 export 调用仓储导出，原字面请求 ID 与原字符串返回；成功 200/no-store，错误 400 且不新加缓存头 |
| 鉴权 | loaded 用原交易凭据，管理接口用原 legacy/separate 管理规则；loopback/IP、提取器优先级及禁播策略不改变这些管理语义 |

没有新增 URL/字段/配置、默认钱包选择、SDK/依赖升级、数据迁移、审计格式或密文格式。

## 源码兼容证据

原仓储本阶段仅把 ManagedWallet 定义替换为 domain 重导出。已将当前源码的这一行在自己创建的临时源码文件中还原为原定义后计算全文件 SHA256，得到上一检查点的精确指纹：

```text
还原后 0609cd75ff6f887fe0c42d1114703e337db5c8f9eb3f58f7c97eb6aaf8aede74
当前   6741458361130d8fd66cd407f3a2adf1d75a63722c1ea53e3613636d25b982b9
```

临时源码随后删除；未改实际源码来做还原。这个比对证明仓储其余字节，包括 SQL、验证顺序、加密、创建/恢复、审计及导出实现未变，不是把文件 hash 变化误报为全部未改。它也不证明原实现所有安全问题已解决。

## 离线验证

新增 11 项 Application、2 项 adapter、4 项 HTTP。fake material 只有 Drop 计数，不是私钥且没有 Clone/Debug；fake export/import 字符串均不是有效密钥。真实临时仓储测试只种公开元数据和无效密文，导入仅走非法字符串早拒绝，导出因无 passphrase 拒绝；不生成/解密/导出真实钱包，不初始化 SDK/RPC、不签名/广播。

- 精确公开列表/八字段 managed JSON、三种创建/导入 201/200 标记、原排序/label/字面身份。
- 原六类仓储错误、加载失败补偿/补偿再失败、默认删除拒绝、remove 部分失败仍保留 disabled。
- opaque material 一次移动/Drop；只有显式导出响应包含 fake marker，成功 no-store，错误不新设缓存头。
- legacy/separate 鉴权、错误/非法提取先于调用、禁播策略不变；实际进程内 Router 接真实 Application + fake ports。
- fake install Future 取消后 material 被 Drop，但原已创建行不被冒充回滚，不新增后台补偿或自动 retry。不能据此声称真实 SDK 的所有 worker 可取消。
- 适配器 Arc 身份、空 loaded registry、脱敏 Debug、真实临时元数据修改与原错误文案/JSON。

相对 F 新增十七项：11 Application、2 adapter、4 HTTP；筛选 18 包含一项既有管理合同。补空串/空格边界后的最终源码验证如下：

```bash
cargo test --locked --offline --quiet wallet_management      # 18 passed
cargo test --locked --offline --quiet                        # 971 passed
cargo test --locked --offline --quiet -- --test-threads=1     # 971 passed
cargo test --locked --offline --quiet http_contract_tests    # 191 passed
cargo check --locked --offline --bin pump-trade-service      # passed
cargo fmt --check                                           # passed
git diff --check                                            # passed
```

production check 仅原 models/persistence 两组兼容导出警告，无新增抑制。旧 V1 hash/JSON、2,016 组 SDK 参数、98 组状态映射、576 组 DAMM 消息和原钱包/余额/WSOL/转账等合同随全套并发/串行通过；不重写黄金期望。fake 成功不代表真实 SDK 加载或生产钱包验收。

## 指纹

wallet registry/ops、executor、PumpAPI、manifest/lock 与 release 保持本阶段前指纹：

```text
162ab6d5c1926bc8378d600a7c5a7f186b8f60c408a7703fa0d768ad65c5f0df src/wallets.rs
c7b0904e48af8e12297dae745fe8625a06e35cb53337afc9adc33956d38e75f0 src/wallet_ops.rs
bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec src/executor.rs
acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b src/pumpapi_client.rs
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

仓储仅定义/别名变更的精确证明见上节；main/AppState、API Handler/DTO、models/domain 与新管理 adapter 有意改变，不能冒称全部旧文件未改。主要接线当前指纹：

```text
66fd021dd9d5459f15fcd33e6d28e03f7f618be8b991e911deaabfc0277e7da6 src/main.rs
d1993d8d34a3afb50d5c390a0b10f095ca41ec81fe55e507f7b0a547bb4b76b0 src/api/handlers/wallets.rs
6ae38f785c7ca41d2dfe0fcd32bcb468e3cb007ae6c04f82a9f2cc3e2ec405f8 src/adapters/sdk/wallet_management.rs
```

## 仍未完成

1. 原取消/部分失败不提供跨仓储与 SDK 的原子激活/回滚保证。此阶段只冻结旧顺序；后续若修复需单独契约与恢复设计，不能把额外 port 当事务。
2. 不透明和 Drop 不等于零化内存、OS 隔离、完整错误脱敏或原 DTO/SDK Config 的全局日志审计。显式 API 导出仍按用户原需求保留。
3. vanity Application/worker 边界、AppState/资源组合、接受入口与依赖方向等继续；真实创建/恢复/导出、生产库、SDK 启动及部署验收仍需单独窗口。
4. NP-01 完整最终消息授权、其他模式/backend 的普通签名账本、原签名对账、SDK 来源/隔离升级准备及全协议验收均未完成，不从钱包分层推导资金安全完成。
5. PumpAPI 交易对接保持暂停；不停止其 ZMQ 或更改已有运行配置。普通 GET 不加入隐式对账/自动下单。

持续目标 active，计划更新和进度汇报不暂停。不升级 SDK、不构建 release、不部署重启、不读生产配置/钱包/数据库、不提交推送、不发送交易。
