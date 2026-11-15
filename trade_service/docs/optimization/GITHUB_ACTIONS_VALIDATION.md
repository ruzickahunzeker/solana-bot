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

- 最新已验收为 `3f37b3e81f7af7a1fc3158e80a7088e86a640447` /
  [Actions 43673491613](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43673491613)，
  artifact `14145199925` 已下载核对。全部步骤 exit=0，完整两线程 1344 / 112.97 秒、
  串行 1344 / 209.88 秒，零失败/忽略/过滤；输出证据 11、预览上下文 13、候选 50、
  probe 59、HTTP 200、参数 3 通过。原 384 ABI 比较与新增 128 拒绝断言实际执行。
  当前新开发 C3b4 的四项编译绑定用例尚未运行，不计入本次通过。
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
