# SDK 来源核对记录

日期：2026-11-12。只读取本地 Git/源码，不访问上游，不修改 vendor。

## 已有证据

| 证据 | 能说明什么 | 不能说明什么 |
| --- | --- | --- |
| 当前 manifest 与 lock | 服务使用本地 vendor 的 `sol-trade-sdk 6.0.2` | 不能证明与官方同版本字节一致 |
| `ed024a4`（2026-09-29） | 本仓库的服务/vendor 初始组织提交 | 不是 SDK 上游基线 |
| `388fd9e`（2026-10-07） | vendor manifest 为 5.0.7，存在 `.cargo_vcs_info.json` | 该提交中其他改动不一定来自该 revision |
| 该旧文件记录 `b5d349d9ab4f9575970336b0d3571c3bed583dc6` | 旧包保留了一个上游 revision 线索 | 不能作为当前 6.0.2 revision，也尚未联网核验 |
| `1baa372`（2026-11-02） | vendor manifest 改为 6.0.2；删除 `.cargo_vcs_info.json`、`Cargo.toml.orig`；217 个 vendor 文件变化 | 大批导入不能还原逐项上游变更/本地补丁归属 |
| 当前 vendor 与 HEAD 无差异 | 本轮未修改 vendor；可用 BASELINE 的 tree 和内容指纹定位 | 不代表本地 vendor 无历史补丁 |

可复核命令（仓库根目录）：

```bash
git log --format='%h %ad %s' --date=short -- trade_service/vendor/sol-trade-sdk
git show 388fd9e:trade_service/vendor/sol-trade-sdk/.cargo_vcs_info.json
git show 388fd9e:trade_service/vendor/sol-trade-sdk/Cargo.toml
git show --stat 1baa372 -- trade_service/vendor/sol-trade-sdk
git diff --exit-code -- trade_service/vendor/sol-trade-sdk
```

## 后续升级门槛

1. 取得可信、固定 revision 的 6.0.2 上游源及校验和，在隔离目录比对；不能拿“最新分支”当旧基线。
2. 分开记录 Cargo 规范化元数据差异、真正源码差异、未知来源差异；不把全部差异自动认定为本地补丁。
3. 每个本地补丁记录文件、目的、影响协议、对应回归测试、上游是否已有等价修复和升级时的保留决定。
4. 对 DAMM 模拟手续费/min_out、CPMM 自定义 Quote、StonkFun 路由、PumpSwap 报价、Token-2022、签名/提交语义逐项保留证据。
5. 新 SDK 只在隔离 adapter/build 中试验；通过合同、故障注入及协议模拟后才进入受控发布。

TS-00 仍为部分完成：本地身份与历史线索已记录，上游 6.0.2 精确 revision 和可重放补丁账本未完成。不得因此直接覆盖 vendor。
