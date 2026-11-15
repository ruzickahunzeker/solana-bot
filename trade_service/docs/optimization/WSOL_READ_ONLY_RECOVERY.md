# TS-S1-G3-C3-B WSOL 原 key 只读恢复

日期：2026-11-13。接续 `WSOL_SUBMIT_ERROR_CONTRACT.md`。仅 Trade Service 源码、临时 SQLite、fake ports、进程内 Router 和文档；没有 SDK/依赖升级、release 构建、部署重启、真实 RPC、签名、广播或生产钱包/数据库访问。

## 为什么单独提供 GET

reservation 已提交但回执丢失时，调用方可能只知道原 idempotency_key，不知道 submission_id。重发 submit 即使在已有记录时只返回 duplicate，也不是只读查询：若保留尚未发生，它可能创建并执行新操作。因此不能以 POST 代替恢复查询，也不根据 400/409/503、超时或断开连接自动换 key。

新增兼容入口，原四个 URL、字段和七字段正常回执保持：

```http
GET /internal/conversions/wsol/submissions/by-key?wallet_id=fixture-wallet&owner_type=telegram&owner_id=42&idempotency_key=original-key
Authorization: Bearer <internal-trade-token>
```

只允许受信任的 loopback + 原 Bearer/IP 鉴权，不能把 owner query 当作公网用户认证。业务调用方仍需验证实际用户，只能传其自己的 owner/wallet 绑定。参数用标准 URL query 编码；key/owner 不应出现在代理或客户端的完整 URL 日志中。

## 项目边界与语义

- API 的 WsolRecoveryQuery 转为项目 WsolRecoveryInput，经既有 WsolQueries façade 和 WsolQueryJournal，最后复用唯一 ConversionStore。没有第二套表、缓存、订单或 reservation。
- 一个 SELECT 同时限定原 key、wallet_id、owner_type、owner_id，先过滤再解码，避免其他 owner 的损坏行成为解码错误探针。Application 再次检查返回记录的四个绑定。
- 原 key 按 UTF-8 字节长度 1..128，trim 只检查非空，不改变字面值；owner/wallet 必须非空。URL 编码、空格、标点、非 ASCII 和字节边界均有测试。不重算/升级 V1 fingerprint。
- 读到记录后校验非空 ID/Quote ID/公钥/指纹、已知状态及原有成对证据政策。半份证据、空签名、脏 reserved 和未知状态拒绝，不修复、删除、解锁或重开 key。
- 返回原七字段快照，duplicate=false 表示查询，不表示创建。signature 可以为空；不公开 key、fingerprint、owner、公钥或交易字节。历史 error 仅在响应副本中做 URL 脱敏，不写回数据库。
- 不读 Quote、不过期校验、不查询当前钱包 metadata、不依赖钱包加载状态，不调用 prepare/sign/simulate/status/broadcast，也不更新观测或时间戳。只要 journal 已配置，即使 Live 关闭或广播禁止仍可读取。
- 它是持久化快照，不是链上最终性证明，也不重新验证历史 artifact 的编码/密码学签名。已有 ID 的 status GET 仍按原签名查询并可 CAS 更新观测；这与本接口的纯 SELECT 不同。

## HTTP 合同

| 情况 | HTTP | x-wsol-error-code |
| --- | --- | --- |
| 找到合法绑定快照 | 200 | 无 |
| key 非法或空白 wallet/owner | 400 | wsol_recovery_rejected |
| key 未找到或 wallet/owner 不匹配 | 404 | wsol_recovery_not_found |
| journal 未配置、读取/解码失败 | 503 | wsol_recovery_unavailable |
| 本 owner 的记录证据不一致 | 503 | wsol_recovery_integrity_error |

应用响应统一 no-store、x-wsol-auto-retry:false、x-wsol-recovery:inspect-original-operation；错误只有一个 error 字段及 x-wsol-error-stage:recovery_lookup，不带操作 ID/签名头，也不发送 Retry-After。身份不匹配和不存在使用相同消息，不泄漏别人的记录是否存在。读取失败不用原始数据库错误作为公开消息。

既有鉴权/Query 提取器/方法拒绝仍用其原合同，不承诺它们包含应用级恢复 headers。未授权或提取失败没有进入 journal。

## 调用方恢复顺序

1. 一次业务意图固定原 key；submit 返回不确定错误或连接中断时停止自动提交。
2. 用原 key、原 wallet/owner GET 本接口。200 则保留原 ID/状态/签名；不根据 reserved/signed/unknown 自动重提。
3. 404 只说明该次读取没有匹配的已提交记录，原请求仍可能在处理/等待提交，不是“没有签名/发送”的证明。保留原 key，调查原请求，不生成新 key 补单。
4. 503 保留上下文，只做只读诊断。拿到 ID 后可用原 status GET 查询原签名，不能自动换路由、blockhash、金额或钱包重签。
5. 人工恢复策略、历史损坏处理和发布准入另作审查，本接口没有提供强制解锁、取消或重广播。

## 离线验证

相对 C3-A 新增 14 项：storage 3、Application 4、真实 Router 7。覆盖 SQL 四字段绑定/字面 key/注入输入、外来损坏行不被解码、文件重开及真实只读 SQLite 连接的所有状态（含 signed/unsigned failed）、Application 单次读取与全部状态/拒绝/坏证据/错误脱敏、真实 Router 的鉴权/方法/Query、静态 by-key 路由、特殊字符编码、回执丢失后文件恢复、请求暂停及取消后的只读查询。

只读连接拒绝实际 UPDATE；多次查询前后行、创建/更新时间戳不变。确定性 gate 证明查询不会使原执行器前进，取消后仍保留 pending 锁；没有用 sleep 推断阶段。最终命令和计数以 `EXECUTION_STATUS.md` 的本阶段结果为准，不以此前 C3-A 的 637 项代替本批验收。

已通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet wsol_recovery        # 14 passed
cargo test --locked --offline --quiet                      # 651 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 651 passed
cargo test --locked --offline --quiet http_contract_tests  # 136 passed
cargo test --locked --offline --quiet wsol                 # 161 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告。Cargo.toml、Cargo.lock、既有 release 的 SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移未修改。

## 剩余任务

C3-A/B 不是完整 G4：原 quote/unsigned simulate/status 的 legacy 错误和历史解码边界仍需独立审查；RPC 诚实性、断电一致性、真实钱包、调用方迁移和生产发布未验收。后续继续 PumpAPI 外来交易审计、普通发送前账本、VerifiedRoute、依赖规则、SDK 来源/升级演练，不增加 Router/SDK 依赖或改默认后端。

后续 G4-A 已补 Quote 解码/行绑定/事务读回，G4-B 又补 status 的身份/形态/单向读回、终态证据与 typed 错误，分别见 `WSOL_QUOTE_STORAGE_INTEGRITY.md`、`WSOL_STATUS_RECORD_AND_ERRORS.md`。by-key 仍是纯 SELECT，以上更新不增加其节点查询、写观测或执行能力。

G4-C 继续补 quote/unsigned simulate typed 错误与 metadata/估算绑定及取消清单，见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md`，仍不改变 by-key 的纯读取性质。
