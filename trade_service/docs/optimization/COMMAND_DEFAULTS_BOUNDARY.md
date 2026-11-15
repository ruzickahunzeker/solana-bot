# NP-03 普通命令默认值配置边界

日期：2026-11-15。状态：源码及受限类型检查完成，四项新增测试尚未执行；未部署。

## 实际变更

- `application/trade_settings.rs` 增加项目自有的 `TradeCommandDefaults`，仅含默认模拟模式和
  旧 PumpAPI 优先费数值；无 RPC、钱包、密文、passphrase、凭据、sender 或 SDK 对象。
- `Config::trade_command_defaults` 只投影这两个原值，不读取环境、不初始化资源、不做新校验。
  这是 outer config 到内层项目类型的单向映射，DTO 不再导入完整 Config。
- 原 `TradeRequest::to_command` 接收 Copy 值快照，保持原 `simulate` 三态解析、显式 tip 的
  `Option::unwrap_or` 行为与 SDK tip/旧优先费默认值独立，不新增 PumpAPI 功能。
- 唯一生产调用仍在原校验/执行政策之后、原 admission/hash/JSON/幂等保留之前；没有改变
  接受顺序、外部 DTO、数量单位、原错误或后台任务/执行 owner。既有合同测试只改调用签名，
  不改原期望/oracle/旧 V1 fixtures。

## 新增四项合同

1. 无完整 Config/SDK/signer 的 DTO 夹具覆盖默认模式两值×请求三态，`u64::MAX` Quote
   原始单位、显式 resolved wallet、字面幂等键及输入 wire JSON 字节不变。
2. 单独费默认值/显式覆盖/正负零的 `to_bits` 一致；NaN/Infinity 默认值只用来核对纯投影
   没有新增提前校验，不代表生产配置或无效请求可绕过原校验。
3. 缺少已规范化买入数量/卖出比例仍返回原命令构造错误，不被默认值校验取代。
4. 合成 Config 的默认值快照不随后续字段修改变化；不相关的 SDK tip/RPC/测试 token 不参与投影。

## 验证与边界

- 受限 test check：`target/dev-checks/20261115T073633Z-140454`，35 秒，
  RSS 2,177,116 KiB，exit=0。未链接或执行本地 Rust 测试。
- 受限 bin check：`target/dev-checks/20261115T073740Z-140817`，13 秒，
  RSS 979,788 KiB，exit=0；原五组生产警告保留，不新增 allow/假调用。
- 后续 CI 增加 `command-defaults` 专项并保留原参数/HTTP/全并发及串行；当前运行
  `43690435964` 的快照不包含本项，不能借用它的结果验收这四项测试。
- Config 仍在 AppState/认证/执行/其他入口；本项只收窄普通命令构造，不宣称全服务配置
  或接受/index/journal/cache/bootstrap 已隔离。剩余基础设施与全方向/模式任务继续。
- 无 SDK/依赖/vendor/release/运行配置/生产钱包或数据库修改，不重启、不实际广播；
  PumpAPI 交易对接保持暂停，仅保留原默认值兼容映射。
