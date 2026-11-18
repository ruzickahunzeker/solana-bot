# SDK 来源核对记录

更新：2026-11-15。已只读访问公开上游固定版本并完成来源/补丁重放核对；未修改 vendor、依赖或 release。

## NP-05-A 固定来源与本地补丁

上游 `v6.0.2` 标签对象 `1dbc0c8bc3a7e04a962130f2e141b1ca071a4a7b` 指向
`dc97b56c602a13d8967973c6eadcf79ab1fcddaa`，tree 为
`6c106a09b8bd04f8541e3aa9db03857db61bdf00`。标签和提交的 GitHub verification
均为 unsigned；这是固定公开仓库快照证据，不是发行者签名认证，也不能把会移动的 tag 当升级依据。

对 API 完整 tree、固定提交源码和本地 vendor 逐文件/执行位核对：上游 372 个文件，
本地 375 个；364 个字节完全一致，8 个修改、3 个新增，没有删除或额外模式差异。
所有差异均是既有 Meteora DBC 扩展：11 文件、486 行新增，不含本次新改动。
manifest 的三行依赖添加是业务依赖差异，**不是 Cargo 规范化差异**。

| 补丁 | 目的与范围 | 升级处置 |
| --- | --- | --- |
| DBC-LOCAL-001 | `Cargo.toml` 添加 bytemuck 与固定 DBC 依赖；client、factory、exports、DexParamEnum 接 DBC；三个新增文件实现参数加载、账户解码/寻池及 swap2 builder | 当前保留；新 SDK 需逐项核对原生 DBC 等价性、ABI/账户/Token Program/min_out/迁移池行为，再决定重放、改写或移除，不能盲目套补丁 |

完整文件/hash 清单与可重放补丁在 `sdk-patches/`。原始上游归档只在忽略的 target；
已核对无路径穿越/符号链接后解压，并核对其全部 372 个文件。
补丁通过 `git apply --check` 并在隔离副本实际重放，375 文件及执行位与当前 vendor 完全一致。
没有在真实 vendor 执行 patch，也没有编译或运行独立 SDK 工作区。

只读检查（仓库根目录）：

```bash
python3 trade_service/scripts/test_sdk_provenance.py
python3 trade_service/scripts/check_sdk_provenance.py
python3 trade_service/scripts/check_sdk_provenance.py \
  --upstream-dir trade_service/target/sdk-provenance/upstream-6.0.2
```

最后一条仅在已取得并隔离解压固定源码后使用。原 SDK 快照必须先通过来源检查，再复制到
`trade_service/target/sdk-provenance/replay`，之后可在仓库根目录执行：

```bash
git apply --check --directory=trade_service/target/sdk-provenance/replay \
  trade_service/docs/optimization/sdk-patches/0001-6.0.2-meteora-dbc.patch
git apply --directory=trade_service/target/sdk-provenance/replay \
  trade_service/docs/optimization/sdk-patches/0001-6.0.2-meteora-dbc.patch
python3 trade_service/scripts/check_sdk_provenance.py \
  --sdk-dir trade_service/target/sdk-provenance/replay
```

检查器不联网、不执行/写入 SDK、不读取环境配置/钱包；增加/遗漏/内容/执行位、符号链接、
不完整清单、旧 hash 不匹配或补丁清单篡改均失败。八项轻量负例测试通过，已加入后续 CI。
补丁目录只对生成的 unified patch 禁用空白检查，因为其空上下文行含必要空格；不影响 Rust、
脚本或其他文档的 whitespace、格式或行为断言。

已有服务回归覆盖 DBC loader、账户 fixture、参数/输入与 HTTP 合同；这些不等于 SDK 自有
工作区测试、真实节点或完整 DBC 买卖验收。DAMM/Temporal、CPMM 和 StonkFun 项目兼容规则
仍须升级演练单独回归，不能因为 vendor 差异只剩 DBC 就删除它们。

NP-05-A 的固定来源/补丁账本与离线重放现已验收。签名来源信任、实际新版本适配/演练、
全协议行为矩阵与真实发布门槛仍未完成；没有因此升级 SDK、修改公开 API 或接入 Router。

## NP-05 Existing Public Surface Follow-Up

Two independently accepted Rust contracts now pin the locked public `DexType` and
factory surface inside the existing SDK adapter tests. The exhaustive match
has no wildcard, covers all twelve current factory variants and checks names
and existing immutable builder singleton ownership without calling `swap`,
creating `TradingClient`/RPC/Keypair or starting workers. This is not a second
runtime registry or a claim that all SDK tasks/resources are supervised.

The service projection is checked against the original fifteen definitions:
nine existing entries, four builder-present/adapter-pending entries, and two
without a claimed builder. `StonkFunSwap` deliberately retains the StonkFun
factory despite CPMM parameters; it must not be silently remapped. Every
observe-only entry must remain rejected as a TradeRequest Pool and expose no
buy/sell/direct-quote/SOL-route permission in both feature-switch projections.
SDK builder presence is not permission or chain acceptance. The test table is
an independent compatibility oracle, never a production lookup.

These two tests are not in final-simulation snapshot `f519207` or its CI. They
were accepted with the coherent configuration batch0530bed/run44363743712/
artifact14457878477: config3/surface2, full2070 each (394.39s/749.17s), all2065
parent names retained plus exactfive,127 zero-exit steps,68types/207 raw fields,
source/locked identity and original initializer reconstruction passed. No test
has run on the small VM.
Future SDK variant/factory/projection changes require explicit
review, not wildcard support or widening V1 capabilities. Existing vendor,
manifests, lockfile, protocol definitions, request projection and API remain
unchanged; no actual SDK upgrade or new protocol activation is performed.

The local provider-conversion follow-up narrows another original SDK facade
boundary, not the supported provider/protocol set. Six existing service provider
configs and eight public choices map to the same pinned SDK variants, strings,
credentials, regional aliases, transports and validation order. Full source
reconstruction and six changed-policy negatives pass. Five provider contracts
and the expanded SDK SWQoS/concrete-cache type gate were independently accepted
in e6c5a6e/run44375225006/artifact14463247130 (full2078 each, provider5/cache3,
129 steps and74types/225 raw fields). First3721 fixture failure was corrected
without changing production methods or old assertions, never rerun.
Seven later SDK controller contracts remain independently pending. Details remain in the existing
APP_STATE_RESOURCE_OWNERSHIP.md, not a second SDK upgrade or rollout plan.

## 2026-11-12 本地核对历史

以下保留当时未联网的记录，其“未核实”由上面的固定版本核对接续，不代表仍待重复调查。

## 已有证据

| 证据 | 能说明什么 | 不能说明什么 |
| --- | --- | --- |
| 当前 manifest 与 lock | 服务使用本地 vendor 的 `sol-trade-sdk 6.0.2` | 不能证明与官方同版本字节一致 |
| `ed024a4`（2026-09-29） | 本仓库的服务/vendor 初始组织提交 | 不是 SDK 上游基线 |
| `388fd9e`（2026-10-07） | vendor manifest 为 5.0.7，存在 `.cargo_vcs_info.json` | 该提交中其他改动不一定来自该 revision |
| 该旧文件记录 `b5d349d9ab4f9575970336b0d3571c3bed583dc6` | 旧包保留了一个上游 revision 线索 | 不能作为当前 6.0.2 revision，也尚未联网核验 |
| `1baa372`（2026-11-02） | vendor manifest 改为 6.0.2；删除 `.cargo_vcs_info.json`、`Cargo.toml.orig`；217 个 vendor 文件变化 | 大批导入不能还原逐项上游变更/本地补丁归属 |
| 当前 vendor 与 HEAD 无差异 | 本轮未修改 vendor；可用 BASELINE 的 tree 和内容指纹定位 | 不代表本地 vendor 无历史补丁 |

可复核命令（仓库根目录）：

```bash
git log --format='%h %ad %s' --date=short -- trade_service/vendor/sol-trade-sdk
git show 388fd9e:trade_service/vendor/sol-trade-sdk/.cargo_vcs_info.json
git show 388fd9e:trade_service/vendor/sol-trade-sdk/Cargo.toml
git show --stat 1baa372 -- trade_service/vendor/sol-trade-sdk
git diff --exit-code -- trade_service/vendor/sol-trade-sdk
```

## 后续升级门槛

1. 取得可信、固定 revision 的 6.0.2 上游源及校验和，在隔离目录比对；不能拿“最新分支”当旧基线。
2. 分开记录 Cargo 规范化元数据差异、真正源码差异、未知来源差异；不把全部差异自动认定为本地补丁。
3. 每个本地补丁记录文件、目的、影响协议、对应回归测试、上游是否已有等价修复和升级时的保留决定。
4. 对 DAMM 模拟手续费/min_out、CPMM 自定义 Quote、StonkFun 路由、PumpSwap 报价、Token-2022、签名/提交语义逐项保留证据。
5. 新 SDK 只在隔离 adapter/build 中试验；通过合同、故障注入及协议模拟后才进入受控发布。

这是历史 TS-00 的缺口记录；精确 revision 与可重放补丁账本现已由 NP-05-A 接续。实际升级/行为/发布仍有独立门槛，不得因此直接覆盖 vendor。
