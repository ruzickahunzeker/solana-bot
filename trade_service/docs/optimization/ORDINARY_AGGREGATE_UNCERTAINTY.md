# NP-01-E0 普通 SOL 聚合不确定回执

日期：2026-11-13。状态：本阶段源码/离线回归通过，未部署。范围：当前 SDK 6.0.2 下的 ordinary SOL aggregator legacy 路径。E0 是完整 C/D/E 广播前账本迁移之前的定向风险修复，不代表完整 E 已完成。仅源码/离线测试，不升级 SDK/vendor、依赖、服务或生产数据，不联网、不执行真实签名/广播。PumpAPI 对接开发保持暂停。

## 原问题与实际修复

修改前先运行 legacy 提交原七项测试，确认其冻结了“广播 Err 后 Unknown/signature=None”的旧行为。随后普通聚合将此结果压成 success=false，普通分类器可进一步显示 failed。RPC 返回另一个签名时旧路径直接显示 submitted；因此原签名诊断与执行结论均可能不准确。

| 情况 | 新行为 | 保持不变 |
| --- | --- | --- |
| 广播返回本地原签名 | Submitted，返回原签名 | 正常成功与提交时点 |
| 广播 Err | Unknown，保留本地原签名，URL 脱敏 | 单次发送，不换 key/路线/后端或补单 |
| 广播返回不同签名 | Unknown，保留本地原签名，提示只核对原签名 | 不采用不同回执替代本地已签交易 |
| Pipeline 明确报告 Live Unknown | 项目内部 typed uncertainty，原签名可为空；不凭错误子串改成 failed | 原 wire 字段/状态枚举、原 key/hash/订单存储格式 |
| 验证/准备/签名前失败及模拟 | 原语义 | 不因 E0 获得 Live 许可或模拟签名能力 |

- `application/legacy_quote_submission.rs` 仅改变签名后广播回执处理，仍没有 journal。
- `application/ordinary_sol_buy.rs` 将明确 Live Unknown 转为 `UncertainSolBuySubmission`；不是通过文字猜测，也没有借用 SDK 专用不确定类型。
- `application/trade_execution.rs` 仅增加这一 typed 分支，原 SDK typed uncertainty 与全部 untyped 98 组状态分类指纹保持。
- `executor.rs`、admission/reservation、V1 codec、持久化 SQL、独立 quote 提交与 SDK/PumpAPI adapter 没有改动；继续由同一原订单记录最终结果，没有第二套幂等权威。
- 原来锁定 unknown→failed 的两个局部用例按这次明确的安全语义更新，不重算或改写任何黄金摘要。

## 离线证据

新增七项测试（876→883）：legacy 回执不一致/错误 URL 两项，Application 缺签名/错误文字/上下文和模拟区分两项，executor 最终写库失败一项，HTTP 重复/冲突/三存储模式与临时文件重开两项；原有丢签名及 unknown→failed 用例更新为新政策。

HTTP 测试实际串联 Router、原接受编排、executor、ordinary SOL buy、legacy prepare/sign/broadcast 编排和原临时持久化；仅末端 backend 为无 Keypair/RPC 的 fake。Fake sign 仅返回固定不可执行样本字节，不进行密码学签名。断言每次首次执行仅一轮报价、fake sign 和 fake broadcast；重复及文件重开后的原请求不再次进入 backend。文件重开仅证明最终订单已写入后的恢复，不冒充发送前落盘。

Application 覆盖原签名为 None/空/正常、错误 None/空/三个旧 failure marker/URL、外层 anyhow context；无签名时仍为 Unknown，不制造默认签名。模拟及其他 outcome 保持原分类。最终数据库关闭测试只证明内存 Unknown/原签名保留且不重复执行，不声称数据库故障后可恢复这些签名字节。

针对性已通过：ordinary_sol_buy 16 项、legacy 9 项、HTTP 聚合不确定路径两项、原 98 组分类指纹一项。最终检查点：

```bash
cargo test --locked --offline --quiet                         # 883 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 883 passed
cargo test --locked --offline --quiet http_contract_tests     # 167 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

check/fmt/diff 通过，仅保留原两组生产兼容导出警告（models、persistence request_hash），无新增抑制。原 SDK 参数/零签名消息/协议/98 组 untyped 状态黄金摘要均未改，随全套通过。manifest/lock/release 与本阶段开始时一致：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

受保护 PumpAPI client/assets、ordinary SDK execution adapter、executor、vendored DAMM 指令文件 SHA 全部保持；本阶段没有修改 vendor、旧迁移或新增迁移，也不触碰其他项目的既有改动。

## 未完成与后续主线

1. 普通订单尚无广播前 signed journal；签名后、最终写库前的取消/崩溃仍可能丢失原签名和字节。不得宣称 exactly-once、原字节恢复或后台 worker 已可取消。
2. 后续普通 journal 必须锚定原 reservation/request_id/key/hash/钱包身份；先确认该锚点再允许签名，原字节持久化精确读回/CAS 后才发送，不复用独立 quote journal 作为第二权威。
3. 最终不可变消息、完整意图/费用/min_out/账户权限授权、同字节受控发送及只读原签名对账、取消/写回执丢失/并发/重启恢复仍须完整实现。
4. 没有追溯重写旧 failed/unknown 订单，没有新建后台重试、轮询或恢复发送任务，不自动重新提交历史订单。
5. 调用方须把 Unknown 当作待核查而非未发送。真实节点/SDK、生产迁移和部署另行验收；本阶段不自行触发这些动作，持续任务保持 active。
