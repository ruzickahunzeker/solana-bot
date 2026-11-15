# C3b4 编译消息与冻结指令绑定

日期：2026-11-15。源码已接候选构建，受限类型检查通过，四项新增 Rust 用例待运行验收。
不升级 SDK、不部署、不操作生产钱包或数据库、不签名广播；PumpAPI 开发仍暂停。

## 实际改动

原 `compile_candidate` 已限制一个 payer/signer 和 1232 字节，但这些检查不足以说明
SDK 编译器保留了输入指令的 program、账户/权限、数据、顺序和 blockhash。
现在 SDK 编译返回后，使用原有标准 Solana `Message::new_with_blockhash` 从同一份
冻结指令构造预期消息，逐字段比较完整 `VersionedMessage`，不只比较长度/签名数或新算的 hash。

这一核对位于真实 `compile_candidate`，因此候选构建、原 SDK full guard、模拟和 metadata
重构都会经过它，不是只存在于测试中的比较。没有复制协议 builder，也没有新增依赖/配置/RPC。

当前版本的 `TradeTransactionVersion::V0` 且无 ALT 时，固定 SDK 6.0.2 产生 Legacy 消息；
该行为此前已由无签名编译合同确认，本轮把它设为该现有 profile 的生产检查。
未来 SDK 即使改为语义相同的 V0 wire，也需要显式 profile/兼容性审查，不在旧记录下自动接受。
这不是宣称所有协议或所有 V0/ALT 都应该使用 Legacy。

## 验证

- 新增四项用例：96 组 provider/模式/比例/Quote/方向/程序矩阵；14 类消息变异；
  旧字节不能绑定新钱包/hash/指令/权限；未经迁移不接受 V0/ALT wire 替换。
- 原测试不删、不改期望指令或消息摘要；原参数/准备、executor、manifest/lock/vendor 均不变。
- test 类型检查约 8 秒，RSS 1,163,716 KiB；bin 类型检查约 3 秒，RSS 547,048 KiB。
  bin 仍七组既有警告，没有抑制或假调用。没有本机大测试链接或 release 构建。
- CI 增加独立 compiled-binding 专项，保留原专项和完整两线程/串行回归。
  C3b3 的 `3f37b3e` / Actions `43673491613` 完整并发/串行各 1344 已通过，
  不包含本轮四项，不能套用其结果。

## 不提供的保证

标准消息编译绑定不证明“冻结指令本身已获得完整业务授权”，不自动认可额外程序或费用，
也不把数据库 metadata 重构变成新鲜的链上证据。现有余额/本笔输出/quote Arc/时间和原 guard
继续生效；完整政策、原 owner 的授权/签名账本、受控签名及原字节发送仍是接续主线。

此检查增加一次标准消息编译，未增加网络读取；尚未以线上测量宣称性能零开销。
