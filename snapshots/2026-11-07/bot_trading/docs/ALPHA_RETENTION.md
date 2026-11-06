# Alpha 存储上限

已安装 bot 源码现对 `alpha_observation_events` 和 `alpha_paper_signals` 设置写入上限，
并在 bot 启动、数据库迁移完成后运行分批清理。bot 服务目前仍停用，现有 SQLite
尚未因本次修改执行迁移或删除。

- 原始观察默认不落盘。显式开启 `ALPHA_PAPER_PERSIST_OBSERVATIONS=true` 时，单条
  `raw_json` 默认最多 4096 字节，表最多 50000 行；满额或超大的原始副本会跳过，
  内存中的 Alpha 评分继续运行。
- Paper 信号的 `features_json` 最多 4096 字节，表最多 20000 行。满额时新信号
  写入返回容量错误，重复 `signal_id` 仍按原记录处理；容量告警每分钟最多一次。
- 后台默认每 900 秒检查。观察保留 72 小时，paper 信号保留 7 天；每轮每类
  删除不超过 `ALPHA_RETENTION_BATCH_SIZE`，有积压时每秒继续一轮。过期清理
  保留仍被 paper 信号引用的观察，硬上限可删除已有 compact signal 的旧原始副本。
  `unknown`、`submitting` 和其它未登记状态不会被清理。
- 每轮提交后尝试 `wal_checkpoint(TRUNCATE)`，以释放 WAL 文件的峰值占用。
  主数据库文件已有的高水位不会因此缩小；如需缩小，需另行备份并安排离线 VACUUM。

配置项及默认值见 [`.env.example`](../.env.example)。只读容量报告：

```bash
python3 scripts/alpha_storage_report.py data/copy_trading.sqlite
```

本次只迁入 Alpha 存储控制；隔离 P4 副本中的 Shadow executor、`alpha_shadow_orders`
和远程兼容 migration 尚未合入。构建与回滚记录位于
`/root/solana/bot_trading_builds/alpha-retention-61bdbf2b120d/` 与
`/root/solana/bot_trading_builds/pre-alpha-retention-20261106T055809Z/`。
