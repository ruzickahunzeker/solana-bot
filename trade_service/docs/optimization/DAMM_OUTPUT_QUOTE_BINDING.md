# C3b2 本笔输出证据与实际候选绑定

日期：2026-11-15。当前源码已接线并通过受限类型检查，运行回归未完成；不是完整签名授权。
接续 C3b1，不重复 metadata/CAS，不升级 SDK、不部署、不接触生产数据或签名发送。
PumpAPI 开发仍暂停；没有修改其文件，复用的既有通用资产校验器也未修改。

## 解决的缺口

旧 DAMM 准备已用八账户检查、本笔 CPI 和模拟银行内的 Token-2022 费用计算净输出，
但只向调用方返回 u64，池/资产/输入/slot/消息上下文丢失。后续重建一个内部自洽的 nominal
profile 仍不能证明它使用同一笔已核验输出，不能仅靠重新 hash/序列化补足。

本轮改为保留原检查与模拟实际产生的私有 `DammOutputEvidence`：

- 沿原八账户检查与原只读模拟产生；不新增账户查询/模拟，不增加 endpoint、依赖或配置。
  保存钱包、池/两侧 Mint/程序/vault、输入 ATA/精度/整数、净输出、原 blockhash 和账户/模拟 slot。
- 生产账户查询指定原余额 snapshot 的 `minContextSlot`，本地也拒绝更旧银行，失败先于模拟。
  原无最小 slot 的逻辑仅作为 cfg(test) 兼容 oracle 保留，实际入口不能传零 slot。
- 年龄从账户检查请求前开始，沿用三秒上限，不在模拟成功/读出/共享/重查时重新计时。
  原余额 snapshot 的三秒起点也不改变；两者都必须有效，重复输出捕获不续查/续期。
- 准备 source 保留 Arc，仍向未修改的 `ordinary_preparation` 返回原 u64 和原参数。
  正式/预览 profile 共享同一 Arc，构造、请求核验、候选/SDK guard 与 preview 后继续检查。
- 比较实际钱包/池/Mint/程序/输入账户/精度/金额/比例与 blockhash；要求证据 slot 不早于余额。
  最小输出必须严格等于原 `minimum_output(net_quote, slippage_bps)`，不使用 gross 或 UI 浮点。
  新的不匹配/过期拒绝使用现有 typed balance failure，不进入 SDK 或换 lane 隐藏该证据错误。

此对象无 signer/RPC/sender，无 Clone/Serde 或公开字段，也不能从数据库 metadata 恢复。
Arc 共享只保留原证据和起点，不授予签名/广播许可。`metadata`/canonical JSON 的重构校验
仍是 nominal 检查，不能通过本轮对象反向宣称持久化数据已经独立可信。

## 验证范围

- 原 `ordinary_preparation.rs`、`ordinary_params.rs`、executor、manifest/lock、SDK vendor 与
  已验收 `6397b0e` 一致；消息/参数/576 与 384 oracle 输入和预期不改变。
- Test 类型检查：临时 2200 MiB/单核 scope，约 9 秒，RSS 1,234,780 KiB，通过。
- 正式 bin 类型检查：约 5 秒，RSS 673,900 KiB，通过；保留原七组警告，无新增抑制或假调用。
- 新增 14 项 Rust 用例已通过类型检查，整体运行验收尚未完成：原始上下文/净值/逻辑 RPC 数、拒绝坏模拟；
  minContextSlot 与提前拒绝；22 项身份/数量/slot/minimum 变异；过期/未来起点/不续期；
  重新构造自洽但错误的池/blockhash/min_out/余额 profile；预览与正式 Arc 共享。
- 旧 nominal fixture 仅增加 cfg(test) 的合成报价对象，供原指令/参数 oracle 构建使用；
  真实数据接线另通过原生产 probe + 无 signer RpcSender 的正/负例验证，不拿合成 fixture 当链上证据。
- fmt/diff 与来源 checker 通过。当前 1321 的完整结果属于 C3b1/NP-05-A，不当作本轮通过。

CI 新增 `damm-output-evidence` 专项，并保留原专项、完整两线程和串行套件。
日志中的“两笔 RPC”只证明逻辑 RpcSender 调用；不声称覆盖实际 wire 重试、流式响应上限或真实供应商。

首轮 `80bd7c1` / Actions `43671447982` 在 DAMM candidate 的 u64::MAX 合成报价夹具失败，
其余后续步骤未执行。夹具修复、保留的 384 ABI 比较和新增 128 拒绝断言见
`DAMM_PREVIEW_CONTEXT_BINDING.md`；不复用旧全绿或弱化生产 guard。

## 尚未完成

1. 整体独立授权仍需资产/池的完整可执行状态、费用版本/预算、附加指令/权限及有效期等证据，
   不把本笔净输出上下文校验扩大为全部安全保证，也不假设 RPC 永远诚实或状态永不改变。
2. 原 owner 下 prepared 的正式编排、授权 CAS、受控签名、签名落盘/独立读回及原字节发送
   仍待继续；正式 Live 仍走原 opaque SDK，不伪造 Quote 或恢复 owner。
3. 当前 probe/模拟仍使用原 SDK RpcClient，隐藏重试、HTTP 缓冲/真实 wire/全局预算和实际总耗时
   门槛未完成；年龄检查只拒绝迟到结果，不等于请求必定在三秒内结束。
4. 买入/其他协议/backend/mode、SDK worker/多签名、生产迁移与真实环境/发布验收仍保留。
