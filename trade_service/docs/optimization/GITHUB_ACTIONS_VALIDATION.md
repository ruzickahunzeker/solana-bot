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

## 已验收运行

最新完整验收 `449731ce21b557421891d82d355a6e2a0a0f1f8a` /
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
