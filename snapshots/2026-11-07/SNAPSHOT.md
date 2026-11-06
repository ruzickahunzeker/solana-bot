# 2026-11-07 Solana 交易项目源码备份

此目录保存上传时的源码快照；原有 `backup/sol-bot-20261102` 分支和本分支的旧版项目目录保持不变。

| 快照目录 | 原机器目录 | 用途 |
|---|---|---|
| `trade_service_candidate/` | `/root/solana/trade_service/candidate/` | SDK 11 Trade Service 候选源码与隔离验收脚本 |
| `trade_service_sdk6/` | `/root/solana/trade_service/legacy_candidate/` | SDK 6 单池隔离修复源码 |
| `bot_trading/` | `/root/solana/bot_trading/` | 本地 Copy Trading / Telegram Bot 源码 |
| `pumpapi_zmq_bridge/` | `/root/solana/pumpapi_zmq_bridge/` | 本地 PumpAPI ZMQ 桥接源码 |

已排除 `.env`、钱包密钥与口令、数据库、运行状态、日志、虚拟环境、Rust 构建输出及其他运行备份。本快照**不能单独恢复钱包或交易历史**；它们应通过单独的加密备份恢复，不上传 GitHub。

SDK 11 的 CPMM 两跳买入与卖出发送代码仍由默认关闭的 `SDK_TWO_HOP_LIVE_ENABLED` 保护。上传时的完整候选门槛通过 167 项 Rust、20 项 Python 测试及隔离 release 构建；最后一次买入 HTTP 模拟回归在任务暂停时中断，不能计为通过。SDK 6 修复版通过 51 项测试与单池模拟，但发送前签名持久化仍未完成。`current` 未切换，18788 未修改，未广播交易。

`MANIFEST.sha256` 列出本快照全部源码文件的 SHA-256。恢复时在本目录运行 `sha256sum -c MANIFEST.sha256`。
