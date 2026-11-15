# TS-04-C Blockhash 只读边界

日期：2026-11-13。状态：源码及 202 项离线测试通过；未部署、未运行实际刷新任务或 RPC。

## 已实施

- Domain 新增项目拥有的 `Blockhash([u8;32])`，不依赖 Solana/SDK，无 API serde；它只是整数定长字节，不是链上有效期证明。
- Application 新增 `LatestBlockhashReader`，只返回 hash，不暴露 signer、交易或 raw RPC。
- `adapters/rpc/blockhash.rs` 持有私有 RPC handles，按原 Default lane 顺序调用 getLatestBlockhash，第一笔成功即停止；失败 URL 继续脱敏。
- `BlockhashCache` 不再持有 SDK/RPC/链类型，只维护 hash、更新时间和刷新循环；通过 port 取得新值。
- Bootstrap 复用 WalletRegistry 原有的 RPC handles 构造 reader，再注入同一个缓存刷新循环；不为每轮新建客户端、钱包或任务。
- 普通交易 Executor 把项目 hash 传给 SDK 参数 mapper；仅 mapper 转成 SDK Hash。SDK 侧收到的 32 字节与原来一致。

## 保留行为

1. 每轮顺序尝试原 lanes，第一成功返回，不新增并行、随机选择、限流或 retry。
2. 一轮查询结束后才 sleep 原 `refresh_ms`；没有改为 interval 或更改启动顺序。
3. 全部失败不清除旧值、不延长旧时间；空缓存仍没有 hash。
4. 缓存年龄按原 wall clock 毫秒饱和相减，精确等于 `max_age_ms` 仍允许；超过才过期。系统时间回退仍按 0 处理，没有偷偷换为新的过期算法。
5. clock 在取得缓存快照后采样，避免提前采样导致锁等待期间低估年龄；空缓存不采样。小型闭包入口只用于可重复测试，生产继续调用原 now_ms。
6. `/ready` 的 has_blockhash、age、ready/paused 合同和缺失/过期交易错误不变。

## 验证

先在旧 SDK Hash/缓存实现上运行 3 项确定性基线：空缓存、新鲜度边界/旧值保留、时钟回退/零年龄。通过后再激活 port/adapter。

迁移后新增 6 项：刷新成功、失败不重置年龄/空缓存不就绪、采样顺序、第一节点成功不消耗备用 mock、失败切备用、空/失败节点不伪造 hash。原 SDK 参数映射回归验证输出 hash 字节一致。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：202 passed、0 failed，无编译警告。新测试只用 fake reader 或内存 RpcClient mock，不启动无限刷新 loop，没有 RPC、私钥或生产配置。

## 仍需单独审查

- 原 getLatestBlockhash 调用无额外 adapter deadline，本轮保持；RPC 超时预算与配额统一应独立设计。
- 原缓存未保存 lastValidBlockHeight/context slot，本轮不改交易有效性政策，也不声称仅凭本地年龄即可证明链上有效。
- WalletRegistry 仍通过 SDK 组装 RPC，其他 Handler 仍有具体 SDK 依赖，整个运行边界尚未完成。
- 原 `/ready` 的 RPC ping 仍取 SDK 执行 lanes，成功 Handler 还需后续只读 port/fake 验证；不因本项通过宣称整服务已可上线。
