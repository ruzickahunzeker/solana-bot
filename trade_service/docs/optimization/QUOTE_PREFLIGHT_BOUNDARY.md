# TS-04-F1 独立报价预检边界

状态：源码与离线回归完成，未部署，未调用真实 RPC 或发送交易。

## 实施内容

- `application/ports/quote_preflight.rs` 只提供已解析钱包的公钥与只读 inspect，不公开 signer、raw RPC、交易对象或提交能力。
- `application/quote_preflight.rs` 负责一次钱包解析、首次缓存检查、预检及第二次缓存检查。
- `adapters/sdk/quote_preflight.rs` 私有保留原 client Arc，复用原 prepare_transaction，丢弃未签名交易，仅返回项目报告。
- HTTP Handler 保留原 POST、query、鉴权、32 字段报告及状态映射；bootstrap 复用原钱包注册表和缓存，不新增 SDK 初始化或配置。

## 不变约束

1. 缺失钱包先返回 400；首次缓存错误返回 410，不执行预检。
2. 准备失败返回原脱敏 502，不检查第二次缓存。
3. 准备成功后使用原钱包句柄再次检查缓存，过期则返回 410 并丢弃报告。
4. 期间注册项删除或替换不触发重新 lookup，不改用新钱包。
5. executable=false 仍可作为只读报告返回 200，不代表批准交易。
6. 缺失 query 仍在鉴权前由提取器返回 400；GET 仍返回 405。

## 验证

新增/迁移覆盖为 service 7 项、空 SDK adapter 1 项和 HTTP 6 项（含迁移前基线）。均使用 fake source、fixture 或空注册表，不执行真实 SDK 初始化、签名或 RPC 模拟。

2026-11-13 再次执行：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet
```

结果：394 passed，0 failed。此前检查点还完成格式检查、串行全套 394 项、HTTP 65 项和 preflight 14 项；这些离线证据不能代替真实节点、部署或资金验收。

## 后续

TS-04-F2/F3 已源码/离线隔离提交与对账装配，详见 `QUOTE_SUBMIT_HTTP_BOUNDARY.md`、`QUOTE_RECONCILIATION_BOUNDARY.md`；报价 Handler 目录迁移见 `QUOTE_API_HANDLERS.md`。submit 保持钱包错误优先、Live 历史重放先于缓存及核心重复校验，reconcile 保持 key-first；签名落盘先于广播、unknown 不重发不变。普通资金签名前账本、完整 VerifiedRoute/可信报价、SDK 来源补丁账本和升级/发布演练仍未完成。
