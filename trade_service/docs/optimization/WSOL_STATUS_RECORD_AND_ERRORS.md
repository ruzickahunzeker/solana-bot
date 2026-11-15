# TS-S1-G4-B WSOL 状态记录与错误合同

日期：2026-11-13。接续 `WSOL_QUOTE_STORAGE_INTEGRITY.md`。仅 Trade Service 源码、离线测试和文档；不升级 SDK/依赖，不构建 release、部署重启、调用真实 RPC、读取生产钱包/数据库、签名或广播。

## 复现与修改边界

两项先失败的假端口测试证明：原 status 能返回不属于请求 ID 的 unsigned 记录，也能接受同身份/签名字节的 unknown -> signed 倒退。进一步补充的一项先失败测试又复现 unsigned failed 在最终读回时被补入签名；修正同时禁止终态 error 被替换。

本阶段只修改 status 读取核对与 HTTP 错误合同，不改变 reservation、签名、模拟、广播、CAS 更新算法、表结构、V1 fingerprint、公开状态字符串或七字段正常回执。quote/unsigned simulate 的错误映射仍保持原样，不把本阶段扩大为所有 API 的错误重写。

## 初始读取

原 store -> ID 查找 -> owner gate 顺序保持。不存在、owner 不匹配仍用原 400 消息；不提前暴露他人的操作 ID/签名。通过 owner gate 后：

- 返回的 submission_id 必须精确等于请求 ID。
- 原八身份必须完整：ID、key、fingerprint、Quote ID、wallet ID、公钥、owner type/ID；key 仍按原 UTF-8 字节 1..128、trim 只检查非空的规则核对，不重写字面值。
- 状态必须已知；reserved 必须无 error/签名/字节；pending/confirmed 需要原成对非空证据；failed 可以保留原 unsigned 或 signed 失败形态。
- 未通过核对不能创建 status reader、查节点、记录观测或发布恢复上下文。

核对复用项目 record 政策，不导入 SDK 类型。by-key 恢复复用同一形态校验，仍是纯 SELECT，不增加节点、当前钱包查询或状态写入。

## 最终读回

继续核对原八身份与已有精确签名字节，并再次核对记录形态。读取并发推进只允许：

| 首次状态 | 最终允许状态 |
| --- | --- |
| reserved | 全部已知且证据形态合法的状态 |
| signed | signed / unknown / submitted / confirmed / failed |
| unknown | unknown / submitted / confirmed / failed |
| submitted | submitted / confirmed / failed |
| confirmed | confirmed |
| failed | failed |

合法的 reserved -> signed、pending -> 最终态并发推进仍可返回；不能把 clean reserved 当作已经签名的证明。终态的签名、字节和 error 还须保持原值，包含 unsigned failed 不得事后添加签名。检查只拒绝不一致的读回，不进行 SQL 修复、删除、解锁或重签。

返回副本中的历史 error 统一 URL 脱敏，原数据库字段不被改写。此核对不是任意全库篡改下的密码学验证，也不会重新裁决首次读取前已经保存的历史终态。

## HTTP 合同与恢复上下文

沿用 C3-A 的单字段 error JSON、错误码及禁止自动重试说明，只增加四个明确阶段：

| 阶段 | 典型情况 | HTTP / code |
| --- | --- | --- |
| status_read | journal 未配置、SQL 读取/解码失败 | 503 / wsol_unavailable |
| status_read | 明确记录不存在、owner 不匹配 | 原 400 / wsol_rejected |
| status_read / status_final_read | 身份/形态/证据不一致、状态倒退或终态替换 | 503 / wsol_integrity_error |
| status_lookup | 原签名 reader 无法构造，如钱包未加载/签名无法解析 | 503 / wsol_unavailable |
| status_observation | 观测写入错误或提交回执丢失 | 503 / wsol_outcome_unknown |
| status_final_read | 最终读取失败或已知记录消失 | 503 / wsol_unavailable |

未知错误文本不能假装成明确参数拒绝；晚到 typed rejection 也不能把观测写入不确定性降为 400。既有三个 HTTP 测试已按这项独立安全变更更新，不称为纯目录迁移。调用方兼容仍需上线前验收，本阶段不修改 Bot。

错误响应使用 no-store、x-wsol-error-code、x-wsol-error-stage、x-wsol-auto-retry:false、x-wsol-recovery；不发送 Retry-After，不授权重提 POST 或换 key。成功七字段回执也增加 no-store。

status 的 ID/签名头只来自通过请求 ID、owner、身份与证据形态核对的首次存储记录，不能取自错误最终回执。初始记录错误/越权/存储不可用时不提供这两项。这里的签名是可信账本的原公开签名上下文，不表示重新执行密码学验签；submit 的签名头仍要求原 clean signed persistence 回执，不因复用 mapper 而放宽。

## 保持的查询行为

status 仍只绑定一个原签名/原 RPC，保留 3 秒 lookup deadline。RPC lookup 错误、缺失、超时或无效观测仍不覆盖交易 error、不自动重查/切节点/重签/发送，最终读取合法原记录后正常返回。只有明确有效 finalized 观测可推进既有 confirmed/failed；非最终错误保持 pending。status 可按原规则 CAS 保存观测，不能把它称为纯数据库只读；需要完全不查询节点/不写观测时使用 by-key GET。

观测提交回执丢失可能已经推进状态，503 不表示未提交或没有结果。文件重开后先读取原操作；已存终态不再查 RPC。任何状态码、暂未查到或 unknown 都不是自动换 key 的许可。

## 离线验证与后续

相对 G4-A 新增 17 项：domain 矩阵 1、阶段分类 1、Application 8、真实 Router 7。覆盖 36 组读取状态矩阵、21 组初始坏身份/形态、最终错误记录/证据、合法并发推进、五类阶段故障、晚到 typed marker、缺失/越权上下文、终态证据/错误不可变、原始 SQLite 损坏行不变、历史 URL 仅响应脱敏，以及观测回执丢失后的文件重开。真实 Router 在禁止广播/Live 关闭、空钱包运行时注入假只读源，不拥有 signer 或发送能力。

最终命令、计数及指纹以 `EXECUTION_STATUS.md` 的 G4-B 检查点为准。此前 659/674 的结果不是最终新增终态测试的验收。

最终以下全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet wsol_status_boundary # 17 passed
cargo test --locked --offline --quiet                      # 676 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 676 passed
cargo test --locked --offline --quiet http_contract_tests  # 146 passed
cargo test --locked --offline --quiet wsol                 # 186 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍只产生 models/persistence 两条既有兼容重导出警告，未用 allow 隐藏。manifest/lock/release SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。数据库损坏 fixture 均在 UUID 临时目录，不是生产历史检查或修复。

下一批 G4-C 继续审查 quote/unsigned simulate 的错误分类、metadata/报价结果绑定及取消/恢复交叉清单，再推进 PumpAPI 外来交易审计和普通发送前账本。完整协议 VerifiedRoute、基础设施依赖规则、SDK 来源/升级演练、真实节点/断电与发布窗口仍未完成，不以本阶段测试通过宣称整体优化完成。

后续 G4-C 已完成上述报价/unsigned 局部工作和交叉证据清单，见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md`。本文件的 676 项为 G4-B 历史结果，不替代后续检查点；真实节点、生产和全服务优化门槛仍独立保留。
