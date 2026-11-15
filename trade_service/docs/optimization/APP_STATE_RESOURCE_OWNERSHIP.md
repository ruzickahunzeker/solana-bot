# NP-03 / TS-09-A 生产 AppState 资源所有权

日期：2026-11-14。状态：源码与离线验证通过，未部署；不是完整 AppState 或基础设施隔离完成。

后续 NP-01-B3a 已完成下述 Quote 签名边界过期复现与最小修复，见 `QUOTE_SIGNING_FRESHNESS.md`；其并发/串行各 989、HTTP 196 不改本页 986 的历史检查点含义。

## 实际变更

- 逐项核对生产读取点后，将 `AppState.wallets`、`wallet_repository`、`aggregator_quotes`、`conversions` 四个旧句柄限定为 `cfg(test)`。普通生产构建不包含这些字段；测试保留原名称以兼容已有夹具。
- bootstrap 仍创建并注入原 registry/repository/cache/store Arcs，未重新建立运行时、缓存、数据库或幂等权威，未调整服务初始化次序。
- 实际请求通过已接入的项目服务使用原资源；四个冗余字段不再作为生产 AppState 的绕过入口。
- full Config、普通 admission/index/persistence、参数/blockhash cache 和项目服务 ports 仍在 AppState。此项不能冒充这些剩余资源或接受入口均已隔离。

## 两项新增合同

`src/http_contract_tests/state_resource_tests.rs`：

1. 替换测试专用字段后，实际 HTTP Router 仍从原钱包服务返回原默认钱包；管理入口仍保留原 File 模式拒绝，而非误用替换的数据库仓储。原四资源在服务持有期间存活，释放整个 Router 后 Weak 均不可升级。只用空 registry、临时库与内存 store，不初始化真实 SDK/RPC。
2. 仅给夹具字段填入 ConversionStore 不会启用未配置 journal 的兑换服务；原 503、阶段/错误码/no-store 保持。最后 Router 释放后该测试 store 也释放。

这些测试证明当前组装的原引用和请求权威没有被替换，不证明实际 SDK 加载、生成、生产关停、后台任务或操作系统资源均已验收。普通生产编译另行核对，不能只靠包含测试字段的 test build。

## 验证

```bash
cargo test --locked --offline --quiet state_resource          # 2 passed
cargo test --locked --offline --quiet                         # 986 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 986 passed
cargo test --locked --offline --quiet http_contract_tests     # 196 passed
cargo check --locked --offline --bin pump-trade-service       # passed
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

本轮 production check 实际观察到五组警告：models/persistence 的兼容重导出，以及 QuoteStore 的 insert/get、WsolQuote 的两个兼容校验方法、旧 SDK wallet_balance 未使用。后三组所涉实现没有在本阶段修改，未用 allow 或删方法掩盖；历史检查点记录的“两组”不能替代本轮实际输出。全套 test build 没有新增警告。

对照前阶段，wallets、wallet_repository、executor、ordinary_execution adapter、vanity worker、wallet_ops、PumpAPI 实现、Cargo.toml/lock、现有 release 二进制 SHA256 均未变化。只修改 main 字段暴露、测试接线/新增测试及本轮记录；未升级 SDK/vendor、构建 release、接触生产配置/钱包/数据库、联网或重启。

## 续作

- NP-01 继续最终可签消息授权、普通原签名只读对账、其他 backend/存储模式的唯一 signed journal；不在现有普通 GET 中悄悄加 RPC，也不自动重签或重发。
- 当前共享聚合签名适配器没有在取出待签消息时重新检查 Quote 有效期；授权期间的异步等待可能使旧检查过期。先补无 signer 的失败复现，再实施最小局部安全修复，不扩大到暂停的 PumpAPI。
- NP-03 后续继续资源/接受入口、依赖方向与兼容桥核查；真实运行验收独立授权。

持续目标保持 active，文档更新不暂停。PumpAPI 交易对接仍暂停，不停止 ZMQ，不改线上配置。
