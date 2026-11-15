# C3b3 预览上下文与边界夹具修复

日期：2026-11-15。持续目标 active。C3b2/C3b3 源码与完整离线回归已验收；
不升级 SDK、不部署、不触碰生产钱包/数据库、不签名广播。PumpAPI 开发仍暂停。

## 预览检查

此前的零签名预览使用固定消息，但只要求响应 slot 非零，没有要求它不早于本笔输出报价；
也没有拒绝响应中的 replacementBlockhash 或不属于该 Legacy 候选的 loaded addresses。

- 生产入口 `simulate_unsigned_at_slot` 必须取得原 `DammOutputEvidence` 的非零模拟 slot。
  请求 `minContextSlot` 指定该值，响应本地再检查；余额到报价到预览的银行上下文不能回退。
- 原交易序列化字节、零签名、单 payer、1232 字节上限、processed、sigVerify=false、
  replaceRecentBlockhash=false、四秒 timeout 均保持，不新增 RPC 或 fallback。
- 拒绝替换 blockhash 和非空 loaded addresses；缺省/空的可选 loaded addresses 继续接受。
- 正式与 preview 不仅须共用原余额，也必须共用同一输出证据 Arc；原三秒起点不刷新。
- 失败、超时、取消、过期或上下文改变均不释放后续 SDK 闭包，无新增真实签名或发送。
  旧无 slot 参数入口只在 cfg(test) 中保留，用于固定旧请求 oracle，不供生产调用。

新增五项 RPC 检查、三项实际 profile/闭包检查；另扩充原过期/slot getter 用例。
RPC 错误脱敏、四秒取消与任务取消的原测试已改走生产有 slot 入口，不减少原断言。

## C3b2 首轮 CI 失败与修复

`80bd7c1` / Actions `43671447982` / artifact `14144671626` 已下载核对。
源码/资源、来源校验、类型检查和测试编译通过；SDK metadata/prepared、参数和 HTTP 200 通过。
DAMM candidate 专项 46 通过、1 失败，随后步骤未运行，不能记为完整 1335 或本轮通过。

失败位于 cfg(test) 的合成报价夹具：原 384 项 SDK ABI 对照包含 u64::MAX 最小输出，
固定 15% 滑点下反推所需净报价大于 u64::MAX，TryFromIntError 被 unwrap。

修复不修改或放宽生产最小输出校验：

1. 保留原三种 minimum、百分比、程序/方向/账户与实际 SDK 指令对照，全部 384 项 ABI 比较保留。
2. 对原 15% + u64::MAX 的 128 项，使用最大可能的 u64 净报价，明确断言 profile 必须拒绝。
3. 另以零滑点合法对照验证同一 u64::MAX 指令/full guard；固定输出 ABI 字节未改，
   不用截断/饱和报价、绕过检查或跳过该边界令测试通过。
4. 新增一项净报价上限负例：15% 下拒绝 MAX minimum、接受正确整数 minimum；零滑点接受 MAX。

新增共九项 Rust 测试，加上 C3b2 十四项，均已在 `3f37b3e` 的完整运行中执行通过。
合成对象只存在于测试夹具；它不替代原生产 probe 的真实账户/模拟证据，也不能当作可签许可。

## 当前验证

- test 类型检查通过：约 8 秒、RSS 1,183,732 KiB；恢复后相同源码再次命中缓存通过。
- bin 类型检查通过：约 5 秒、RSS 629,056 KiB；仍是原七组警告，无抑制或假调用。
- fmt、diff 和固定 SDK 来源检查通过；未在小 VM 链接测试目标。
- CI 保留旧专项、完整两线程与串行套件，增加预览上下文专项，并把输出证据放在 ABI 专项前。
- `3f37b3e` / Actions `43673491613` / artifact `14145199925` 已下载并核对全部步骤 exit=0：
  完整两线程 1344（112.97 秒）/串行 1344（209.88 秒），零失败/忽略/过滤；
  输出证据 11、预览上下文 13、候选 50、probe 59、HTTP 200、参数 3 全部通过。
- 原 384 SDK ABI 对照和 128 不可能 minimum 拒绝断言实际执行；没有跳过原边界或运行完整套件。
  新的 C3b4 编译绑定不在该提交内，不能套此结果。

## 后续主线

本轮只收紧原预览校验，没有把 preview 误称为包含 relay tip 的正式消息模拟。
正式 Live 仍是 opaque SDK；完整独立授权、原 owner 的 signing/signed/receipt CAS、
受控签名、签名落盘/独立读回和原字节发送仍须继续。

已只读核对固定 SDK 6.0.2：`SwqosClientTrait::send_transaction` 接受已构建的
`&VersionedTransaction`、TradeType 和 wait_confirmation，不要求 payer；
当前 GenericTradeExecutor 则经独立 worker 构建/签名再调用它，worker 可超出调用方取消范围。
后续可评估复用 sender 而不复用该 worker，但必须先核对 provider 内部 fallback/重试、
tip eligibility、编码与确认语义；这些源码发现不是原字节 sender 已接线的声明。
