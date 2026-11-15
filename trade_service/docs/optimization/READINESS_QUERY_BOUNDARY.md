# TS-04-D Readiness 只读查询与真实路由合同

日期：2026-11-13。状态：源码和 213 项离线测试通过；未构建 release、未部署、未启动监听器、未加载真实钱包或查询真实节点。

后续检查点：以下为 TS-04-D 当时保留的原探测/采样政策。2026-11-14 RPC-01B2 已明确调整内部探测预算、去重与缓存采样时序，公开 V1 合同不变；原 probe/三项测试原字节保留为 oracle，见 [有界 Readiness 探测](READINESS_SHARED_RPC_RUNTIME.md)。本文“无 deadline/先采样/私有 RPC handles”等描述不作为后续源码现状。

## 边界

- Domain 新增公开钱包描述、ReadinessSnapshot/Report/钱包 lane 元数据；字段为项目 Address、布尔值、整数和字符串，无 Axum/SQLx/SDK/私钥。
- Application 新增 `ReadinessSource` 只读 port 与 `ReadinessService`；唯一 V1 ready 规则仍为 `has_blockhash && rpc_ok`。
- `ReadinessQuery` 作为 HTTP 组装入口返回一个 boxed future，使真实路由可注入 fake source；仅诊断请求存在这次分配，不涉及交易热路径，不添加依赖或 async_trait。
- `RuntimeReadinessSource` 通过私有 Arc 复用已有 config、registry、cache、persistence，不反向引用 AppState，不新建 SDK、RPC 或钱包。bootstrap 组装一次。
- SDK 的原 ping_rpc 被只读 probe 替代：probe 只有公开元数据与私有 RPC handles，使用原 `execution_lanes()` 结果，不持有 signer 或 SDK client。
- `/ready` Handler 鉴权后只调用 `state.readiness.inspect()` 并做 DTO 转换，不再取默认 SDK 钱包或 raw RPC。
- wire ReadyResponse 移至 `api/dto/readiness.rs`，原 models 导入路径重导出。钱包列表通过项目拥有的 WalletDescriptor 转换，原 ID 排序保持；不建立另一套钱包身份缓存。

## 保留合同

1. JSON 的 17 个字段、名称、类型、null、列表顺序及 HTTP 200 语义不变；IP/Bearer/separate 管理鉴权不变。
2. `ready` 仅表示本轮取样的 blockhash + RPC 条件。`paused=true` 或 `trade_database_healthy=false` 不自动改写 ready，mode 也不改变该公式。
3. 生产仍先采样 has_blockhash，再取得默认钱包的公开 probe 并顺序 ping；之后读取当前钱包列表、age、配置和 DB 健康。
4. probe 中的钱包/lane 元数据来自原 SDK 不可变属性；动态钱包 count/list 仍在 RPC 后读取，未宣称多个字段原子快照。
5. RPC 仍使用原执行 lanes，包括 MEV 优先/require_mev_submit 选择；第一成功停止，不新增并行、去重、备用策略、额外 timeout 或缓存。
6. RPC 全失败为 rpc_ok=false，DB 故障为独立 healthy=false；不在纯提取中把它们换成新的 HTTP 错误。

## 冻结与测试

激活之前已运行旧 ReadyResponse 的完整合成 fixture：`tests/fixtures/contracts/readiness_v1.json`。它只代表 wire 合同，两个“钱包”是公开测试地址，不是可交易钱包或链上验收。

新增 11 项测试：

- 1 项旧 DTO 完整 17 字段基线。
- 7 项使用实际 `http::router`、Tower oneshot、可注入 source 的成功/失败鉴权测试：完整旧响应、48 组 ready/paused/DB/mode 组合、每个布尔字段独立映射、age null/u64::MAX 精度、鉴权失败零探测、管理 token 不能查询、钱包/lane/count 的独立值与顺序。
- 3 项内存 RPC probe 测试：第一成功不消耗下一节点、失败后顺序使用下一 lane、空/全失败返回 false，公开元数据原样保留。

HTTP 测试的 WalletRegistry 仍为空，默认查询对象会在意外执行时 panic；成功 readiness 测试必须显式注入没有 SDK/signer 的 source。若 Handler 回退为直接访问 SDK 默认钱包，真实路由的成功测试会失败，不需要构造假 SDK 钱包。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：213 passed、0 failed，无编译警告。无生产 .env、钱包、RPC、数据库访问；SDK vendor、manifest/lock 和原 release 不变。

## 剩余风险

- Runtime source 仍私有持有 WalletRegistry 等现有组件；这提供消费者只读接口，不等于整个服务不持有密钥或所有 raw RPC 已隔离。
- 原 RPC ping 没有额外 deadline，且可能对同一 URL 的多个 SDK lanes 重复查询。本轮不调整频率/配额。
- has_blockhash 与 age 在不同时间采样；慢 ping 之后原 ready 可能不代表此刻仍足够新鲜。缓存只看 wall-clock 年龄，未引入 lastValidBlockHeight。改变这些政策应单独设计/发布。
- 尚未以真实 SDK 初始化、节点和钱包验证 runtime source 的成功链路；当前证据是原 selector 源码复用、独立 mock probe，以及真实 HTTP Handler 的 fake source 合同。
- 成功下单 Handler、发送计数与持久化故障注入仍未因本项完成；下一个入口按同样方式逐步引入受限查询/执行接口。
