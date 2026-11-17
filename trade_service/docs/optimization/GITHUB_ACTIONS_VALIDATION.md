# GitHub Actions 验证边界

日期：2026-11-15。用户选择以 GitHub Actions 验收，并完成浏览器设备授权；仅使用源码和公开测试数据。
目标仓库为 `ruzickahunzeker/solana-bot`，不是其他 bot 仓库。

## 发布与输入

- 本地优化基线与远端 main 没有共同祖先，且 SDK 基线不同；不合并、不改写或强推 main。
  使用独立、无历史的 `ci/trade-service-validation-*` 源码快照分支，仅供验证当前本地 Trade Service。
- 独立临时索引不改变工作区 main 或真实索引。只纳入 Trade Service 源码、其原 vendor、manifest/lock、
  文档、公开 fixtures、验证脚本和本工作流；另含相关优化方案。Copy/Bridge/sdk9 修改不进入快照。
- 不纳入 `.env`、passphrase、钱包 JSON/密文、数据库、日志、backup、target 或编译产物。
  路径白名单、常见敏感字面量扫描与 CI preflight 是防线，不把模式扫描声称为通用密钥证明。
- OAuth 凭据只在本机标准 gh 配置，权限 600，不进入源码、日志、workflow secrets 或 artifacts。
  CI checkout 不持久化仓库凭据，job 只有 `contents: read`。

## 验证过程

1. GitHub-hosted Ubuntu 22.04，固定 Rust 1.95.0，与本地编译器一致。
2. 检查源码输入、可用内存/磁盘，下载锁定依赖；不自动更新 Cargo.lock 或 SDK。
3. 创建临时 network namespace，仅开启 lo；setpriv 降回 runner 身份，env -i 丢弃外部配置/凭据。
   之后所有编译/测试为 `--offline`。实际 probe 确认回环可用、无默认路由、文档地址不可达。
4. 顺序执行格式、workflow policy、namespace probe、shell runner、SQLite DDL、类型检查、编译测试、
   SDK prepared/候选、参数/HTTP、DAMM/probe/lanes 专项、完整两线程和串行套件。
5. 不 build release、不启动服务、不连接真实 RPC/Telegram、不注入任何生产密钥或数据库。
6. artifacts 只允许生成的 logs、identity.txt、results.tsv；不上传 target 二进制、临时 SQLite 或钱包。

云端是独立冷构建环境，关闭依赖的 debug/incremental 以控制空间，与本机复用缓存的规则不同；
不改变 SDK 版本或 release 行为。新旧参数/消息/合同必须真正运行通过，不能因构建地点变化更新 oracle。

## 当前批次

后继 `7a63477` / Actions `44132188285` 已因源码发现冷配置 180 ms/pacing 冲突主动
取消，artifact `14351441124`；不计完整验收、不重跑。新源码以同次六账户快照投影
解决，新增隔离冷成功见证/错 commitment 等断言，仍为同 23 个新增名称；不降低
250 ms pacing，不改 SDK/decoder/全局缓存，不沿用旧快照结果。

首轮参数快照 `255ef09` / Actions `44130661475` / artifact `14348964516` 已终态失败：
十项前置通过，测试类型检查 221 秒发现新夹具缺导入/私有方法访问。已用显式测试导入
和 cfg(test) request/reply helper 修正，不改生产可见性或断言。该批未运行 Rust 测试/
102 字段门禁，不复用父批结果、不重跑旧失败；后继源码单独验收。

2026-11-18 原 NP-03 参数读取 scope：源码23项新增测试（含1隔离 helper）、三项新
专项与102字段门禁待独立CI。小 VM受限metadata检查触及内存上限，不再本地重试；
导入/可见性警告已修，最终类型/运行须本批CI。要求保留父批1652名称，增量恰为23，
完整并发/串行与所有门禁独立核对。见 `PROTOCOL_PARAMETER_SHARED_READS.md`。
下方presence批次为已验收历史，不套用给当前源码。

2026-11-17 续原 NP-03 的普通买入 presence 接原 preparation 预算，十四项新增合同
及新 `preparation-presence` 步骤已独立验收。父批 1638 名称全保留且增量恰为十四项，
完整并发/串行原始名称相同；原源码/99 字段拒绝/恢复及全部专项已核对，不重复旧运行。
详见 `PREPARATION_PRESENCE_SHARED_RPC.md`；下方 882e514 是已验收父批，不套用给当前。
本批 `27bd9969a5a725547ae22a0c97701030be9b30aa` /
[Actions 44115951258](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44115951258)
终态 success，artifact `14343274443` 已下载：全部 57 步骤 exit=0；完整并发 1652
（168.50 秒）/串行 1652（296.55 秒）零失败/忽略/过滤。presence 13、实际买入 runtime
5 通过；原始 compiler spans 的 99 个独立字段拒绝、原源码/恢复成功及 identity/
manifest/lock/vendor manifest 均独立核对。不覆盖真实节点/供应商或生产验收。
审计绑定 1066 文件/13,089,071 字节、20 白名单路径，保留真实 main/索引、冻结历史、
manifest/lock/vendor/迁移；记录本身不再触发文档 CI。

2026-11-17 原 NP-01 受控买入执行接线：十七项新合同，新增 controlled-buy、buy-claims
和 original-buy-runtime 专项；同一 Clippy 策略增两个私有买入产物，32 类型/99 scoped
字段（含外部 alias），不借用父批 93。完整并发/串行及原始诊断已按本批身份独立
下载核对，不重跑任何终态任务。

前批已验收 `882e514a499df01c68c704671ecdf335755e8ea9` /
[Actions 44109233896](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44109233896)
终态 success，artifact `14340653473` 下载独立核对：全部 56 步骤 exit=0；完整并发
1638（149.17 秒）/串行 1638（275.68 秒），零失败/忽略/过滤，名称集合相同、父批
1621 全保留且新增恰为十七项。controller 买入 6、买入 claim 3、实际买入 runtime 4、
executor 10、sender 15、扫描 17、买入 probe 79 均通过；原源码/恢复 Clippy 成功，
原始 compiler spans 独立确认全部 99 个不同字段拒绝。identity 与 manifest/lock/
vendor manifest 匹配。不代表真实节点/供应商或生产验收。
源码发布审计：1062 文件/13,058,154 字节；后继四路径仅测试及记录修正。真实 main、
索引、冻结历史、manifest/lock/vendor/迁移保持；不单独为运行记录触发文档 CI。

首轮 `b572b7a` / Actions `44102178151` 已终态 failure，artifact `14336409343`
下载核对：27 步通过，controlled-buy 6、buy-claims 3、sender 15 通过；99 字段门禁
通过。executor 9/10，新增夹具错误地要求 blockhash 过期先于钱包解析，原实现顺序
相反。仅修正断言并补重复调用不进入 SDK/legacy 的检查；完整买入 runtime/两套
完整回归未执行，不套用父批结果，不重跑该终态任务。后继修正源码另发原验证分支。

后继 `ebc51d4` / Actions `44104753013` 已终态 failure，artifact `14339127559`
下载核对：前 29 步通过，executor 十项/原卖出 runtime 三项通过；新增买入 runtime
四项失败于夹具旧消息索引。源码复核确认原参数工厂开启 rate-limiter sysvar，旧夹具
未开启；不是完整消息才加入的 Compute Budget 差异。`76f898d` / Actions `44107876296`
的索引映射尝试在本地复核后主动取消，不计验收、不重跑。后继夹具使用相同 profile，
逐字核对实际 discovery 消息而非重排索引，保留生产同 bank 余额、输出及权限检查；
受限 metadata check 13 秒/RSS 1,427,180 KiB 通过。
后继 882e514 已独立完成上述完整验收，不重跑失败/取消任务，不套用父批。

## 已验收运行

前批原 NP-01 买入消息/签名字节证据、原意图绑定及历史回放
`f029d55c1778f0688015d85d29dab7562146ee39` /
[Actions 43785820055](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43785820055)
终态 success，artifact `14190956041` 下载独立核对：新专项 18（新增十六项及原两项）、
共同记录 9、买入 79，完整并发 1621（120.77 秒）/串行 1621（227.60 秒）零失败/
忽略/过滤，全部 53 步骤 exit=0。两套原始名称集合相同，父批 1605 全保留且增量
恰为源码十六项；原源码/恢复 Clippy 成功，原始 JSON spans 独立确认三层 93 个不同
字段拒绝。identity 与 manifest/lock/vendor manifest 匹配，发布审计绑定 1059 文件/
12,989,654 字节、30 路径。旧 SellV1、claim/sender/扫描 SQL、迁移、SDK/vendor、冻结
历史、真实 main/索引保持。只证明对应源码的原意图及历史证据校验，不等于生产受控
买入 signer、owner/CAS/落盘发送或自动扫描已接完；原 claim 仍闭，不部署、不重跑
终态任务，后续验收文档不单独触发 CI。平台 Node 20→24 的固定 action 提示与父批相同，
任务实际成功，不顺带调整 SDK 或依赖。

最新原 NP-01 原 journal 的买卖记录类型边界 `2fe436dff1326b5ce0d069804df2e4ba3ad6b07b` /
[Actions 43781625542](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43781625542)
终态 success，artifact `14189296618` 下载独立核对：共同记录专项 9、买入专项 79，
完整并发 1605（142.55 秒）/串行 1605（261.47 秒）零失败/忽略/过滤，全部 52 步骤
exit=0。两套原始名称集合相同，父批 1594 全保留、增量恰为源码十一项；原源码/恢复
Clippy 成功，原始 JSON spans 独立确认三层 93 个不同字段拒绝。identity 与 manifest/
lock/vendor manifest 指纹匹配，发布审计绑定 1052 文件/12,938,550 字节、35 路径。
旧 SellV1 原字节、claim/sender/扫描 SQL、迁移、SDK/vendor、冻结历史、真实 main/
索引不变。本批只证明共同记录/原 journal 类型接口及方向拒绝，不等于买入实际 owner
授权、落盘签名/发送或恢复完成，不扩大 Live；不部署、不重跑终态任务，验收记录
不另发文档 CI。固定 actions 的 Node 20→24 平台提示与父批相同，任务实际成功。

最新原 NP-01 买入独立名义记录 `19f80cb00f2816341791a359d4504bc5fe83ea1d` /
[Actions 43777721549](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43777721549)
终态 success，artifact `14187333877` 下载独立核对：买入专项 79（含新增 11），完整并发
1594（140.68 秒）/串行 1594（257.60 秒）零失败/忽略/过滤，全部 51 步骤 exit=0。
两套原始名称集合相同，父批 1583 全保留、增量恰为源码十一项；原源码/恢复 Clippy
成功，原始 JSON spans 确认三层 93 个不同字段拒绝。identity 与 manifest/lock/vendor
manifest 匹配，发布审计绑定 1047 文件/12,913,244 字节、16 路径；旧 SellV1、Application/
claim/journal/sender/迁移、真实 main/索引/冻结历史保持。终态任务不重跑，记录更新不另
触发 CI。仅源码/离线数据格式及候选绑定验收，不等于原 owner、落盘、签名/发送或恢复。

最新原 NP-01 买入整笔候选/回调与无签名模拟 `ec1d2c9b1569802e245239d0c0437261be578a0e` /
[Actions 43773807696](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43773807696)
终态 success，artifact `14185728930` 下载独立核对：买入专项 68（含新增 19），完整并发
1583（155.62 秒）/串行 1583（274.91 秒）零失败/忽略/过滤，全部 51 步骤 exit=0。
两套原始名称集合相同，父批 1564 全保留、增量恰为源码十九项；原源码/恢复 Clippy
成功，原始 JSON spans 确认三层 93 个不同字段拒绝，包括新增两种买入类型及外部别名。
identity 与 manifest/lock/vendor manifest 指纹匹配，发布审计绑定 1044 文件/
12,875,578 字节、21 路径；真实 main/索引/冻结历史保持。终态任务不重跑，后续记录
不另触发 CI。仅源码离线验收；买入原 owner 账本/固定 sender、最后回调后 worker/
时效及真实供应商/生产验收仍未完成，不据此扩大 Live/profile 或恢复暂停的 PumpAPI。

最新原 NP-01 买入净报价/min_out 准备绑定 `61af682d81a52b69db891ebf12226095c7004cd6` /
[Actions 43734656743](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43734656743)
终态 success，artifact `14170038159` 下载独立核对：买入专项 49（含新增 16），完整并发
1564（152.42 秒）/串行 1564（268.72 秒）零失败/忽略/过滤，全部 51 步骤 exit=0。
两套原始名称集合相同，父批 1548 全保留、增量恰为源码中的十六项；原源码/恢复 Clippy
成功，原始 JSON spans 独立核对三层 87 个不同字段拒绝。identity 与 manifest/lock/vendor
manifest 指纹匹配，发布审计绑定 1039 文件/12,820,838 字节、19 路径；真实 main/索引/
冻结历史保持。不重跑终态 CI，后续验收记录不单独触发 CI；仅源码离线验收，不等于
最终买入消息授权、买入原 owner 账本/固定 sender、SDK 入场后时效或真实供应商/发布验收。

最新原 NP-01 买入同步/预充值兼容 `fca54ff9a87581822502707bd9226a5a917c2786` /
[Actions 43730145561](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43730145561)
终态 success，artifact `14168537736` 下载独立核对：买入专项 33（含新增 10），完整并发
1548（138.47 秒）/串行 1548（251.95 秒）零失败/忽略/过滤，51 步骤 exit=0。两套
原始名称相同，父批 1538 全保留、增量恰为十项；原源码/恢复 Clippy 成功，原始 spans
逐项核对 87 个不同字段拒绝。identity/manifest/lock/vendor manifest 匹配，审计绑定
1037 文件/12,779,254 字节、12 路径，真实 main/索引/冻结历史保持。此证据不覆盖
后继 actual ordinary buy 准备/min_out 接线，不重跑终态 CI 或因文档记录另触发 CI。

最新原 NP-01 买入只读资金/净输出原型 `5e0ed8f64bcdfc8b0b5b4cdd61a4051df41114fe` /
[Actions 43727995203](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43727995203)
终态 success，artifact `14167687550` 下载独立核对：买入专项 23（含新增 16），完整并发
1538（138.18 秒）/串行 1538（252.38 秒）零失败/忽略/过滤，全部 51 步骤 exit=0。
两套原始测试名集合相同，父批 1522 保留、增量恰为十六项取证合同；原源码/恢复 Clippy
成功，原始 spans 逐项确认 87 个不同字段拒绝。identity/manifest/lock/vendor manifest
匹配；原审计绑定 1036 文件/12,760,389 字节、11 路径，真实 main/索引与冻结历史保持。
此证据不覆盖后继同步/预充值兼容源码、正式准备/min_out/授权或真实节点验收；不重跑
旧任务，不因验收记录更新另触发 CI。

最新原 NP-01 买入 ABI/funding 原型 `48f15b3300e0206132d771bb7807dd1cfb63eef6` /
[Actions 43725621186](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43725621186)
终态 success，artifact `14166866773` 下载独立核对：专项七项含 72 组 SDK builder
指令对照，完整并发 1522（147.65 秒）/串行 1522（258.80 秒）零失败/忽略/过滤，
51 步骤 exit=0。两套原始测试名集合相同、父批 1515 保留、增量恰为这七项；原源码/
恢复 Clippy 成功，原始 spans 的 87 个不同字段拒绝逐项匹配。identity/manifest/lock/
vendor manifest 匹配，发布审计绑定 1034 文件/12,713,954 字节、12 路径；真实 main/
索引/冻结历史不变。此批仅 cfg(test) 原型、合同/CI 与文档，不等于完整报价取证或
买入 min_out 保护，后继源码必须独立验收；终态运行不重跑，验收记录不单独触发 CI。

最新 NP-03 原 Shadow writer 生命周期 `796ba5037149db4c3511aff7c3c7ae80f83cbc67` /
[Actions 43722582055](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43722582055)
终态 success，artifact `14165837512` 下载核对：专项 13/4 共 17，完整并发
1515（147.21 秒）/串行 1515（258.54 秒）零失败/忽略/过滤，50 步骤 exit=0。
两套原始测试名集合相同，父批 1498 保留，增量恰为 17 项专项；原源码/恢复 Clippy
成功，原始 JSON spans 独立确认 87 个不同字段拒绝，三层各 29，包含 outside_alias。
identity/manifest/lock/vendor manifest 匹配；发布审计绑定 1032 文件/12,691,338 字节、
17 变更路径，SQL helper/recovery 与原持久化连接设置未改。真实 main/索引不变；
未部署、未改线上配置，不将 FIFO SQL 完成等同 fsync/断电/完整服务退出验收。
后续只更新文档，不重跑本任务或因验收记录另触发一次 CI。平台的 Node 20→24
action 运行时提示与父批相同，任务成功；不因此顺带改 SDK 或依赖。

最新 NP-03 普通任务生命周期 `c6f2745e2fee315d454b0c7327d1469f6c1fb441` /
[Actions 43718342603](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43718342603)
终态 success，artifact `14163942665` 下载核对：专项 12/4/1 共 17；完整并发
1498（134.03 秒）/串行 1498（244.95 秒）零失败/忽略/过滤，48 步骤 exit=0。
两套原始测试名集合相同，父批 1481 全部保留，增量恰为 17 项；原源码/恢复 Clippy
成功，原始 JSON spans 独立确认 87 个不同字段拒绝，含新运行时两类型与 outside_alias。
identity/manifest/lock/vendor manifest 匹配；审计绑定 1029 文件/12,654,972 字节、
25 个变更路径，真实 main/索引不变。未部署/启用新配置，不重跑旧任务；发布后的验收
和 503 原 key 说明不单独触发文档 CI。普通任务生命周期不等同全服务/SDK worker/
Shadow 落盘或生产停机验收。

最新 NP-04 双假构建合同 `91b3d94037a1d557ce2b255f4eb491bc308e01ac` /
[Actions 43714527309](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43714527309)
已终态 success，artifact `14162686991` 下载核对：新增专项八项、完整并发
1481（129.52 秒）/串行 1481（236.54 秒），45 步骤全部 exit=0。两套原始测试名
集合相同且包含全部八项新增；原源码/恢复成功，原始 JSON 独立核对 81 字段拒绝。
身份/service manifest/lock/vendor manifest 匹配，发布审计绑定 1022 文件/
12,615,602 字节，真实 main/索引不变。本批仅测试/runner/文档，生产源码未改；
不等同真实 Router/SDK、其他业务路径或部署验收。旧任务不重跑，后续验收记录不单独触发 CI。
本次平台提示固定 checkout/upload-artifact 的 Node 20 运行时已被 runner 强制到 Node 24；
任务实际成功。该 action 维护提示不涉及 SDK/vendor 升级，本批不顺带更新依赖。

最新已验收类型/别名门禁 `e3be4420fe68049f5f5ce399405682620d44ee17` /
[Actions 43711006653](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43711006653)
已终态 success，artifact `14161032189` 下载核对：44 步骤全部 exit=0，完整并发
1473（144.04 秒）/串行 1473（253.36 秒）零失败/忽略/过滤；工具三项、Python 门禁十项/
workflow 七项及原专项通过。原源码/恢复 Clippy 成功，negative exit=101 是预期；另独立
读取原始 JSON 诊断确认三个层共 81 个不同字段均被拒绝，含外层别名，不仅依赖摘要。
身份/manifest/lock/vendor 匹配，原发布审计绑定 1017 文件/12,572,454 字节；真实 main/
索引不变。没有合并或部署，不重跑旧任务；本地后续仅整理文档，下一源码批再发布。

首轮类型/别名边界 `8bd77968dce15ac2985808a88614d6b78780949c` /
[Actions 43710108477](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43710108477)
已终态 failure，artifact `14159823162` 下载核对：全部前置/服务类型检查及工具三项
通过。Clippy 子命令未收到全局 --config，离线 zap 来源解析失败，未到真实类型检查，
81 负例和完整回归未执行。后继修正参数转发并增加零依赖真实 Cargo 回归，
保留离线、源替换、lock/vendor 和全部原断言；修正已由 e3be442 验收，不重跑旧失败任务。原审计绑定 1017 文件/
12,568,938 字节，真实 main/索引不变，不套用父批。

前序完整验收普通执行编排 `26445c597c75d273bf7a8a7264da9cc33192fb16` /
[Actions 43707853050](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43707853050)
已终态 success，artifact `14159283766` 下载核对：完整并发 1473（112.34 秒）/串行
1473（209.87 秒）零失败/忽略/过滤，ordinary-runtime 七项及全部步骤 exit=0。
identity/manifest/lock/vendor 匹配，真实 main/索引不变；原审计绑定 1010 文件/12,531,337 字节。
后继类型门禁/小型开发工具另由上方 e3be442 独立验收，不套用本结果或重跑旧任务。

前序完整验收普通接受快照 `26be71781406be7f912d53bd26e04fb549dce363` /
[Actions 43706312318](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43706312318)
已终态 success，artifact `14159195795` 下载核对：完整并发 1466（129.94 秒）/串行
1466（238.29 秒），零失败/忽略/过滤，ordinary-intake 八项、HTTP 202 与全部步骤 exit=0；
identity/manifest/lock/vendor 匹配。1006 文件/12,503,715 字节审计只绑定本快照。
后继 ordinary-runtime 七项及完整编排另由上方 26445c5 独立验收，不套用 1466。

首轮普通接受流程 `c58eb8cfdc68e72bb1b9a67bb34e90cae69f6c3e` /
[Actions 43705256866](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43705256866)
已终态 failure，artifact `14158646075` 下载核对：所有前置/原专项及 HTTP 202 通过，
ordinary-intake 五项通过/三项失败，完整并发/串行未运行。失败为新增夹具误读 Shadow
异步镜像、旧 command oracle 固定错误钱包；现仅修正测试输入和有界读取，原 fixture 与
字段/字节断言保留，不改生产实现。修正已在后继 26be717 验收，不重跑此旧任务。
原 1006 文件/12,497,881 字节审计只绑定 c58eb8c，真实 main/索引未变。

前序完整验收 readiness `c220ba879db0b6fee766b22919cdfce6f8803f5d` /
[Actions 43696813225](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43696813225)
已终态 success，artifact `14154584905` 下载核对：完整并发 1458（143.31 秒）/串行
1458（252.76 秒）零失败/忽略/过滤，readiness-settings 六项和全部步骤 exit=0。
本轮再次查询该句柄确认为 completed/success，不重复触发。后续普通接受流程及八项新合同
已发布到上方新快照，ordinary-intake 专项已加入 runner，不能套用本批 1458。

前序完整验收 bootstrap `970d8350a6508bbebcf2ebbbbae7b09bebfa7b8c` /
[Actions 43695116074](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43695116074)
已终态 success，artifact `14154666415` 下载核对。完整并发 1452（143.42 秒）/串行 1452
（253.37 秒）零失败/忽略/过滤，bootstrap-assembly 4/HTTP 201/参数 3 与全部 steps exit=0。
identity/manifest/lock/vendor 指纹匹配，七组生产警告保持，999 文件/12,438,464 字节审计通过，
真实 main/索引不变；readiness 后批六项不在此快照，不套用本结果或重跑旧终态任务。

前序完整验收 `cb94bdd1d5b54ffd6566e722d92529e06d2f3718` /
[Actions 43693519994](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43693519994)
已终态 success，artifact `14153732230` 下载核对，完整并发 1448（142.77 秒）/串行 1448
（251.64 秒）零失败/忽略/过滤，api-access-policy 4/validation-settings 4/HTTP 201/参数 3
与全部 steps exit=0。identity/manifest/lock/vendor 指纹匹配，七组生产警告保持；996 文件/
12,421,585 字节审计通过，真实 main/索引不变。后续 bootstrap 四项不在本批，不能套用结果。
旧终态任务不重跑，也不在本机重试大链接。

前序完整验收 `449731ce21b557421891d82d355a6e2a0a0f1f8a` /
[Actions 43691952757](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43691952757)
已终态 success，artifact `14153850007` 下载核对；完整并发 1443（133.72 秒）/串行 1443
（236.47 秒）零失败/忽略/过滤，command-defaults 4/param-cache-port 2 与全部步骤 exit=0，
identity/manifest/lock/vendor 指纹匹配，七组生产警告保留。994 文件/12,403,750 字节审计通过，
真实 main/索引未变；后续鉴权/校验配置五项不在本批，不能套用本结果。旧终态任务不重跑。

前序完整验收快照 `6c50aca628313a8683b84800adbba1b72e751cd3` /
[Actions 43690435964](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43690435964)
已终态 success，artifact `14152368482` 下载核对，identity/manifest/lock/vendor 身份匹配，
全部 steps exit=0。完整并发 1437（128.56 秒）/串行 1437（235.46 秒）零失败/忽略/过滤，
runtime 5、扫描 16、历史迁移 1、HTTP 200、参数 3 通过。990 文件/12,383,832 字节审计通过，
真实 main/索引不变；后续默认值/缓存 port 六项不在本批。下方 43688840398 为历史失败，
其夹具已由本后继修复验收，旧终态任务不重跑。

最近终态 C3d2 `57c75c5f39b59a55685dbc4fefcc7b906ab246d8` /
[Actions 43688840398](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43688840398)
已终态 failure；artifact `14152165576` 已下载核对，sqlite-sdk-scan、sdk-scan 16 与全部
前置步骤 exit=0，完整并发 1431 通过/1 失败（128.17 秒），串行未运行。失败为旧 005
历史结构测试在删除 final_observation_json 前没有删除新 010 索引，不是扫描专项失败。
当前只修测试基线重建，补 010 重建/唯一 marker 断言，不放宽生产迁移。C3e 新增五个入口
仅类型检查，待新 CI；增加独立 observation-migration/sdk-original-runtime 步骤，保留全套。
不要重新轮询或重跑这个终态失败任务；新快照身份另行记录。

最新已验收 C3d1 `87f86775c533ab18306abed634c1827eb02e98ec` /
[Actions 43686886493](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43686886493)
已终态 success，artifact `14150979478` 下载核对，identity/manifest 指纹匹配，所有步骤 exit=0。
完整并发 1416 / 129.44 秒、串行 1416 / 229.33 秒，零失败/忽略/过滤；恢复 16、controller 12、
sender 14、HTTP 200、参数 3 通过，原 18 项新增已实际执行。当前 C3d2 的 16 项扫描专项
已通过，独立 DDL 六项已通过，但完整 Rust 验收未过，不混用 1416。
下方 C3d1 in_progress 是发布时历史，不再等待或重跑这个终态任务。

当前待验收 C3d1 `87f86775c533ab18306abed634c1827eb02e98ec` /
[Actions 43686886493](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43686886493)
已推送并确认 in_progress，新增 sdk-recovery 专项并保留原全部专项/完整两线程和串行套件。
只跟踪此新句柄；当前 18 项新增用例无运行结果，不借用下方父提交的 1398。

最新已验收 C3c2 `fd1c7cb8869d670cffbd3c5485744b7e8666f773` /
[Actions 43685088919](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43685088919)
已终态 success，artifact `14150028402` 下载核对，manifest/SDK 身份匹配，全部步骤 exit=0。
完整并发 1398 / 132.54 秒、串行 1398 / 235.13 秒，零失败/忽略/过滤；sender 14
包含四项真实受控 signer/原 SQLite/实际 RpcClient 的串联用例，controller 10、HTTP 200、参数 3 保持。
当前 C3d1 新增 18 项恢复/finality 用例仅受限 check，待新快照 CI，不套用 1398。

以下 C3c2 in_progress 为发布时历史，不继续等待或重新触发这个已终态任务。

当前待验收：C3c2 `fd1c7cb8869d670cffbd3c5485744b7e8666f773` /
[Actions 43685088919](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43685088919)
已确认 in_progress。仅新增四项 journal integration 与文档，不混用父提交 1394 作本批通过证据。
后续只跟踪这个句柄；下方 C3c1 成功为最近完整验收，不再作为运行中任务等待。

- C3c1 `dd946aaa961f821a000db492fd8c74381180695f` /
  [Actions 43683583741](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43683583741)
  全部步骤成功（14 分 32 秒），artifact `14149574413` 已下载核对：身份/manifest 指纹匹配，
  results.tsv 所有步骤 exit=0；完整并发 1394 / 124.26 秒、串行 1394 / 225.17 秒，
  零失败/忽略/过滤。prepared 22、签名账本 11、controller 10、受控签名 5、sender 10、
  executor 分流 8、选定 observer 2、HTTP 200、参数 3、候选 54、probe 59 均通过。
  先前过期 fixture 已执行验证修复，无测试删除或跳过；production 五组原警告保留。
  后续 C3c2 四项 journal integration 仅类型检查，尚未由该提交验收；不部署/升级或访问生产数据。
  固定官方 Actions 的 Node20→24 弃用提示已记录，非测试失败；后续单独核对可信官方新 pin，
  不为消除此提示移动依赖/权限边界或使用浮动 actions 标签。

以下为历史运行记录，不覆盖上方最新已验收结果，也不授权重跑旧终态任务。

- 当前 C3c1 真实 SDK 原 owner 执行接线及 20 项新用例已通过受限类型检查，完整验收待新 CI。
  C3b7 `83036a4` / Actions `43680602908` 已失败，artifact `14147664555` 已核对：
  candidate 15、prepared 22、signed journal 11、controller 10 通过，受控签名 4/5，完整套件未执行。
  过期夹具的 cfg(test) 修复不修改生产构造器/时效限制，见 `SDK_ORIGINAL_DAMM_RUNTIME.md`。

- 当前 C3b7 的五项真实受控签名用例、原控制器及完整回归待新 CI。
  C3b6 `bd39f04` / Actions `43678564856` 已失败，artifact `14147911636` 下载核对：
  类型/测试构建、candidate 15 通过，prepared 21/1；controller 与完整套件未执行。
  失败夹具的约束拒绝保留与隔离 reader 注入修复见 `SDK_CONTROLLED_SIGNING.md`，不削弱生产约束。

- 当前 C3b6 源码已有应用编排/SQLite port、十项新用例和 C3b5 修复，受限类型检查通过，
  运行结果待新的源码快照 CI。C3b5 `3bd979b` / Actions `43676492857` 终态 failure，
  artifact `14146766179` 已核对：类型/测试构建通过，candidate 14/1；后续专项与全套未执行。
  阶段读取合同的生产修复和原断言保留见 `SDK_SUBMISSION_APPLICATION.md`；不把失败隐藏或跳过。

- C3b4 编译绑定快照 `b1fbc8693d9e3da6589c15b855666703f2c9e35d` /
  [Actions 43675181497](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43675181497)
  成功，artifact `14145658342` 下载核对：全部步骤 exit=0；完整并发 1348 / 114.52 秒、
  串行 1348 / 213.36 秒，零失败/忽略/过滤；compiled-binding 4、候选 54、prepared 11、
  HTTP 200、probe 59、参数 3 通过。当前新增 C3b5 的 11 项 SDK 签名账本用例尚未运行，
  须另发新源码快照，保留全部专项和完整两线程/串行套件；不冒用旧 1344 结果。

- 前序已验收为 `3f37b3e81f7af7a1fc3158e80a7088e86a640447` /
  [Actions 43673491613](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43673491613)，
  artifact `14145199925` 已下载核对。全部步骤 exit=0，完整两线程 1344 / 112.97 秒、
  串行 1344 / 209.88 秒，零失败/忽略/过滤；输出证据 11、预览上下文 13、候选 50、
  probe 59、HTTP 200、参数 3 通过。原 384 ABI 比较与新增 128 拒绝断言实际执行。
  当时不包含 C3b4 的四项编译绑定用例，已由上方 1348 接续。
- C3b2 首轮 `80bd7c1` / Actions `43671447982` 的夹具溢出是历史失败，已由上述运行接续。
  修复与新增负例见 `DAMM_PREVIEW_CONTEXT_BINDING.md`，不是删除该失败边界。
- [Actions 43667516925](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43667516925)，
  commit `35e436296cebcc2edda99bc7faa541f64217c758`，全部 job 步骤成功，约 15 分钟。
- 已下载并核对 artifact `14143511883`：身份、白名单路径和所有 results.tsv 行 exit=0；
  完整两线程 1321 / 123.08 秒、串行 1321 / 221.71 秒，均零失败/忽略/过滤。
- 专项：SDK candidate 15、prepared 11、参数 3、HTTP 200、DAMM candidate 43、probe 48、lanes 19。
  Rust all-target 类型检查 238 秒，test 冷构建 226 秒；没有 skip 完整套件。
- policy 六项、锁定依赖四项、SQLite DDL 六项、shell runner、实际 namespace probe 均通过。
  七组生产警告原样保留，其中两组说明 prepared helper/方法仍未接正式 SDK，不伪称接线完成。
- 运行只用了源码/公开 fixtures；manifest/lock/vendor 未被 CI 修改，没有 release、部署或生产网络访问。
  本机锁文件与原 release 哈希也保持不变。
- 后续来源 gate 也已验收：[Actions 43669338197](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43669338197)，
  commit `6397b0eb81884e134a2dcdb213a409a61045a611`、artifact `14144266219` 已下载核对。
  source checker/八项负例、policy 七项及所有步骤 exit=0；完整两线程 1321 / 123.73 秒、
  串行 1321 / 222.43 秒，零失败/忽略/过滤。这是 C3b2 改动之前的验收基线。

## 历史诊断与设施

- 第三轮 `452dce7` / [Actions 43666935608](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43666935608)
  进入 Rust 类型检查，发现 Cargo vendor 未包含 Anchor 宏读取的 workspace 级 Jupiter IDL。
  补同一原 Git 提交的单份 IDL 并固定 Git blob/SHA-256，未修改 SDK 或协议指令。
- 第二轮 `ad4ea9e` / [Actions 43666705603](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43666705603)
  锁定依赖获取、格式、policy 五项、依赖源码三项及仅回环 probe 通过；shell runner 因 runner
  未安装 rg 而失败，仍未进入 Rust 测试。后续显式安装 ripgrep/time，并增加一项安装策略检查。
- 已推送独立分支 `ci/trade-service-validation-20261115-c3b`，首个提交 `ffc506e`。
  首轮 [Actions 43666176316](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43666176316)
  输入/资源检查通过，但锁定的上游 zap-program 下载返回 404/认证失败，未进入 Rust 测试。
  后续只对 CI 使用同一提交的 Cargo vendor 三包源码与文件校验，不改变 manifest/lock/SDK；
  来源和范围见 `../../ci/README.md`，不向 CI 注入本机 OAuth 或其他 Git 凭据。
- 官方 checkout/upload-artifact 的固定提交已通过 GitHub API 核实。
- 本地 workflow policy 五项通过；临时 namespace probe 确认仅回环、无外部路由。
- 本机没有运行大 Rust 测试编译；完整验收以实际 Actions run/commit 与结果为准。
- 上面的已验收运行才是当前完整结果，前三次失败是历史诊断。后续 CI 若失败，应修正对应源码并重跑同一验证链，
  不删断言、不开放外网或 Live、不自动合并/部署。
