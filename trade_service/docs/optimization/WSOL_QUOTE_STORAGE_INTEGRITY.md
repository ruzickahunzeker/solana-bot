# TS-S1-G4-A WSOL 报价存储完整性

日期：2026-11-13。接续 G3-C3-A/B。仅 Trade Service 源码、离线测试和文档；没有 SDK/依赖升级、release 构建、部署重启、真实 RPC、签名、广播或生产钱包/数据库访问。

## 复现与范围

在临时 SQLite 将 quote_json 改为 BLOB 后，旧 get_quote 的 row.get::<&str> 会 panic；进程内 spawn 返回 JoinError，不能把它当成可控业务错误。另有 JSON/列类型解码错误未被标为完整性异常，JSON 的 quote_id/expiry 与行键/expiry 也未被一起核对。先加入两项失败测试复现，再修正源码。

本阶段没有新增字段、表、迁移、hash 或报价算法。旧金额、13 字段 JSON、60 秒 TTL、source balance/fee/rent 校验、七字段回执和签名/广播顺序保持。它保护存储来源和读回，不代表报价已经通过链上验收。

## 读取

- 显式读取 quote_id、quote_json、expires_at_ms，使用 try_get，异常作为 Result 返回，不调用会 panic 的 row.get。
- JSON 和列解码失败统一 typed Integrity，公开消息固定为 stored WSOL quote could not be decoded。不将原始 JSON/未知枚举值/损坏载荷拼进错误，也不保留其原始错误链作为公开上下文。
- 查询键、行键、JSON Quote ID 必须相同；行 expiry 必须是可转换的非负整数且精确等于 JSON expiry。失败返回 stored WSOL quote identity or expiry does not match its row。
- 缺少行仍返回 None；历史过期 Quote 仍可读，不能在存储读取时施加当前准入规则而破坏同 key 历史查询。
- 历史空白格式及未知 JSON 字段仍按原 serde 规则读取，不重排、重写或删除原字节。其他 Quote 业务校验仍由 Application/SDK 受控授权完成，不把 JSON 可解码当作可签名许可。

## 写入

save_quote 仍使用原 serde JSON 和 i64 expiry，现将 INSERT 与读回校验置于同一事务：

1. 插入必须影响一行；忽略插入、SQL 故障不返回成功。
2. 按原 Quote ID 读取，校验行/JSON 元数据及全部 13 字段。
3. 新写 JSON 原始字节必须等于原编码，不能接受 trigger 对新写入的空白/字段改写。
4. 只有完整读回成功才提交；异常返回后回滚，不发布一份与实际保存内容不一致的新 Quote。

重复 ID 仍拒绝，不 upsert、覆盖或生成替代 ID；u64 expiry 无法转换为 i64 时仍在写入前拒绝。表结构、WAL/FULL、唯一 ConversionStore 和 V1 fingerprint 不变。新写严格读回与历史 JSON 宽容读取是两个边界，不对既有行做格式迁移。

## HTTP 与恢复

submit 的 quote_lookup 收到 typed Integrity 时，沿用 C3-A 返回 503/wsol_integrity_error 和禁止自动重试说明，无尚未验证的操作 ID/签名头，且不进入 reservation/runtime。不能把这项证据推广到其他 503 或并发原操作。

quote POST 和 unsigned simulate 仍按既有 legacy 400 映射，不借本阶段扩大错误接口迁移；真实 Router 测试固定了此区别。quote 保存失败不会返回 executable/quote 回执；事务内对其他既有 Quote 的 trigger 副作用也一起回滚。

by-key 恢复只读 conversion_submissions，不依赖 Quote 可解码、尚未删除或仍有效。保留提交回执丢失后，即使 Quote 已损坏，也能读取原 reserved/ID，不重提 POST、不调用节点、不释放 pending。

## 离线证据

本阶段新增 8 项：storage 5、真实 Router 3。覆盖损坏 BLOB/TEXT/JSON/expiry、行与 JSON 的 ID/expiry、插入忽略/失败/删除/篡改/字节变化、全部 13 字段逐一变更、重复 ID、expiry 溢出、旧格式与未知字段兼容、只读文件池重开且不改写坏历史、submit/simulate 不同错误合同、quote 写回滚与受损 Quote 下的原 key 恢复。

最终全套、串行、HTTP、WSOL、目标筛选、fmt/diff/check 和指纹结果见 `EXECUTION_STATUS.md`。复现时的两个预期失败不替作最终通过证据，也不把 C3-B 的 651 项结果替作本批验收。

以下全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 659 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 659 passed
cargo test --locked --offline --quiet http_contract_tests  # 139 passed
cargo test --locked --offline --quiet wsol                 # 169 passed
cargo test --locked --offline --quiet wsol_quote_storage   # 8 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍只有 models/persistence 两条既有兼容重导出警告。manifest/lock/release 的 SHA-256 分别仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。

## 剩余审查

- G4-B 已单独补 status 初始/最终记录与终态证据核对，以及状态读取/观测错误分类，见 `WSOL_STATUS_RECORD_AND_ERRORS.md`；G4-C 已接 quote/unsigned simulate typed 错误、metadata/估算校验和取消/恢复交叉清单，见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md`。上文 G4-A 的 legacy 400 和 659 项是当时检查点，不代表当前映射。
- 历史坏行只拒绝，不自动修复、删除或重签；没有验证任意整库篡改下的密码学完整性。
- RPC 诚实性、断电/刷盘延迟、真实钱包与调用方迁移、生产发布仍需独立窗口。后续 PumpAPI、普通发送前账本、VerifiedRoute、依赖规则及 SDK 来源/升级演练仍未因此完成。
