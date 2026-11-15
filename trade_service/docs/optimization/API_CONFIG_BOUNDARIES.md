# NP-03 API 鉴权与普通校验的窄配置边界

日期：2026-11-15。源码及受限 test/bin 类型检查通过，五项新增 Rust 合同尚未执行，未部署。

## 实际接线

- API 三个鉴权函数只接 `ApiAccessPolicy` 借用视图：原交易 token、原 IP 列表和原
  WalletAdminAuth 引用。31 个现有 Handler 调用已切换，原 HTTP 身份来源仍为 ConnectInfo。
- 视图不克隆 token/列表或读取环境，不持 Config、钱包、passphrase、数据库、RPC、SDK
  或 signer；没有 Debug/Serialize 输出。Config 到视图的纯投影只发生在原鉴权位置。
- 旧 `trade_client_ip_allowed` 规则原样移到视图：loopback 或原精确 allowlist 匹配。
  IPv4-mapped IPv6 不被额外归一化；不信任 x-forwarded-for，不调整代理或监听配置。
- 交易入口仍先 IP 再 token；internal 先 loopback 再原交易鉴权；Legacy admin 保持原分支，
  Separate admin 仍仅 loopback + 独立管理 token。所有原错误码/正文/日志事件及检查顺序保持。
- 普通交易校验只接已有 `TradeValidationSettings` 的五值投影；执行准入直接调用原
  Application `ensure_execution_allowed`，仍在 normalize 之后、command/admission 之前。
  Config 版 validate_trade/validate_execution_policy 仅 cfg(test) 保留给原兼容夹具。
- 不改默认值、amount alias/整数单位、Wallet ID/公钥映射、拒绝持久化、V1 hash/JSON、
  幂等或执行逻辑；不新增或启用 PumpAPI 交易能力。

## 五项新增合同

1. 独立视图夹具覆盖交易端口 IPv4/IPv6/allowlist/未授权 IP、Bearer 格式及错误/缺失 token，
   核对 IP 拒绝先于 token；二进制 header/空 token 保持拒绝。
2. internal 对已 allowlist 的远端仍拒绝；伪造 forwarded header 不能代替 socket peer。
3. Legacy/Separate 管理模式、internal 标志、两类 token 与两种 loopback 的准入/错误顺序。
4. 合成 Config 到视图的三个引用逐项指针/长度相等，清空合成 RPC/钱包配置不影响纯投影。
5. 五值 validation settings 快照保持浮点 bits/原默认数量/滑点/负零及未用 NaN；投影不增加
   eager 校验，原无用配置不影响显式字段。无效值测试不是放宽生产环境解析或交易校验。

已有 HTTP/参数/V1/拒绝顺序合同不改期望；新增 api-access-policy（四项）与
validation-settings（原三项加一项）专项，并保留全并发/串行。前序 `43691952757`
虽已完整并发/串行各 1443 通过，但不包含本批，不能用它当作本批验收。

## 当前证据与剩余

- 最终 test check：`target/dev-checks/20261115T080153Z-150488`，约 10 秒，
  RSS 1,338,228 KiB，exit=0；无新增测试警告。
- 最终 bin check：`target/dev-checks/20261115T080309Z-150888`，约 4 秒，
  RSS 670,156 KiB，exit=0；沿用前批七组生产警告，不抑制/假调用。
- 本机不链接/运行 Rust 测试；完整运行放在用户已授权的源码专用隔离 GitHub Actions。
- AppState 仍持完整 Config，Handler 仍从原状态取得 flags/等待参数；接受/index/journal/
  cache/bootstrap、剩余 SDK/RPC 资源与全方向/协议/模式仍待继续。不把窄鉴权参数称为完整
  资源隔离或 OS 级秘密隔离，也不改变线上权限配置。
- SDK/依赖/vendor/release 身份不变，无生产钱包/数据库操作、部署/重启或实际广播。
