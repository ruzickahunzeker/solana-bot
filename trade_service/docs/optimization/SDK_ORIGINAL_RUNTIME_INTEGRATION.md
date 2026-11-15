# C3e 原 SDK 完整准备与受控发送离线串联

日期：2026-11-15。仅新增测试及测试专用 fixture 出口，不改生产执行行为、API、SDK 或依赖。
五个 Rust 测试入口已由 `6c50aca` / Actions `43690435964` 实际运行通过；artifact
`14152368482` 下载核对。完整并发 1437（128.56 秒）/串行 1437（235.46 秒）零失败/忽略/过滤。

## 范围与实际调用

`SdkTradeClient::execute_original_damm -> LanePreparation -> SharedReadRuntime`
`-> actual SDK pool/mint loader -> zero-signature DAMM output probe -> unsigned preview`
`-> Application journal/permit -> actual wallet_signer/controlled signing`
`-> original signed bytes persisted/read back -> OriginalSdkSender -> actual RpcClient`。

- 新 Durable Live SDK 直接 DAMM v2 卖出，Default 单 sender；不代表其他模式/方向/协议或
  provider 已验收，不覆盖部署或真实节点能力。
- 使用同步 `TradingClient::from_infrastructure`，不是会预热/创建 WSOL ATA 的异步构造器。
  测试前断言没有 SDK RPC；metadata-only provider 的所有 send 方法 panic，禁止高层 SDK worker。
- 准备余额实际经回环 HTTP，scan/raw 使用当前临时钱包与规范 ATA；SDK 参数加载、probe、
  preview 和提交使用实际 RpcClient，但底层为严格匹配方法/参数的内存 RpcSender。
  隔离子进程清除代理；CI 无外网，无真实凭据、链上签名提交或交易钱包。
- 余额 1000 原始单位，卖出 10%=100，bank 123，probe slot 124 净输出 120，
  滑点 1500 bps 后最小输出 102；最终 metadata、原 request hash/JSON、claim 不变均有断言。
- 两次模拟均断言零签名、同一 supplied blockhash/临时 payer；preview 和最终签名消息逐字节相等。
  默认 V0-without-ALT 按锁定 SDK 产生 Legacy 消息，不改变原 oracle。
- 提交出口核对真实 SQL 行已 broadcasting、签名与 message/packet 原字节一致、Ed25519 可验、
  原 send 配置一致，才返回 typed receipt；成功只发送一次。

## 故障用例

1. raw Mint owner 错误、probe 输出零、preview 模拟错误：无签名落盘、无发送；原请求不变，
   原 claim 不能重取。分别断言 SDK RPC/模拟数量，避免从错误阶段提前失败却误判覆盖。
2. 返回其他签名：保留原签名并 unknown；receipt 落盘失败：原广播证据及父行 submitting
   保留，不把传输成功当作账本成功。两者都恰好一次发送，不重建或重发。
3. probe 或 send 阻塞后取消：实际未来被 drop；分别保留 claimed 无签名或 broadcasting
   有签名；再次 claim 为 AlreadyHeld，短调度检查无额外 RPC，不产生新 owner。

这些测试只验证可控内存传输/回环 HTTP 下的项目与 SDK 调用链，不证明 TLS/HTTP2、
真实供应商接受语义、内部重传、生产额度、崩溃刷盘、真实池报价或全量安全授权。

## 当前证据与下一步

- 类型检查：`target/dev-checks/20261115T072730Z-136412`，36 秒，RSS 2,154,948 KiB，exit=0。
- 初始 private module、私有 pool 和 Tokio/std Mutex 类型错误均以 cfg(test) 出口/测试自有
  pool handle 修复；不放宽生产字段可见性。
- CI 的 `sdk-original-runtime` 五入口及 `observation-migration` 一项均通过，后者保留
  原数据/父行/claim 并新增 010 索引/marker 恢复断言。sdk-scan 16、HTTP 200、参数 3
  与全部 steps exit=0；identity 中原 manifest/lock/vendor 指纹匹配。
- 后续 NP-03 命令默认值/缓存入口六项新增合同不在该快照，不能据此宣称它们已运行。
- 后续仍须其他方向/协议/模式、剩余资源与 backend 边界、升级演练及独立真实环境验收；
  无 SDK 升级、生产迁移、服务重启或实际广播授权。
