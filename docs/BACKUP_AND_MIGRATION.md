# Solana Bot 备份与迁移

## GitHub 内容

仓库只保存可审查的源码、SDK vendor、方案文档、systemd 模板和 `.env.example`。以下内容禁止提交：

- `.env`、Telegram Token、RPC/MEV Key、`MASTER_KEY_HEX`
- 私钥、钱包 JSON、钱包数据库和导出文件
- SQLite 运行数据库、日志、ZMQ spool、`target/` 编译目录
- 包含交易历史或用户数据的备份归档

## 敏感备份

钱包和运行配置使用独立的加密归档保存，不上传 GitHub。当前备份范围：

- `trade_service/data/wallets.sqlite`
- `trade_service/data/trades.sqlite`
- `trade_service/data/conversions.sqlite`
- `trade_service/.env`
- `copy_trading/.env`

17GB 的 `copy_trading/data/copy_trading.sqlite` 按当前迁移约定不保留；如未来需要审计，应单独归档到加密存储，不放入代码仓库。

### 2026-11-02 归档说明

历史归档 `sol-bot-sensitive-20261102.tar.enc` 使用创建时本机
`copy_trading/.env` 中的 `MASTER_KEY_HEX` 作为 OpenSSL 加密口令；它不是
`trade_service/.env` 的配置项，也没有另存一份独立归档密码。解密时使用：

```bash
openssl enc -d -aes-256-cbc -pbkdf2 -iter 100000 \
  -pass file:/secure/copy-trading-master-key \
  -in sol-bot-sensitive-20261102.tar.enc | tar -xvf -
```

其中 `/secure/copy-trading-master-key` 必须由管理员从原始
`copy_trading/.env` 安全提取，不能提交 GitHub 或与归档放在同一目录。
迁移完成后应轮换 `MASTER_KEY_HEX`，并按新的密钥重新加密钱包数据库和备份。

## 新服务器恢复顺序

1. 拉取 GitHub 仓库，确认工作区没有 `.env`、数据库或私钥文件。
2. 安装 Rust、SQLite、systemd 运行依赖，按各项目 lockfile 安装依赖。
3. 从加密备份恢复 `trade_service/.env` 和 `trade_service/data/`。
4. 恢复 `copy_trading/.env`，检查 `TRADE_SERVICE_URL`、Bearer Token、Telegram 白名单和 ZMQ 地址。
5. 先启动 Trade Service，验证 `/health`、`/ready` 和 `/v1/capabilities`。
6. 再启动 Copy Trading 和 PumpAPI Bridge。
7. 先执行 dry-run/模拟测试，确认钱包 ID、公钥和能力门禁，再开启 Live。

## 安全要求

- 加密归档密码不能和归档放在同一台服务器或同一个 GitHub 仓库。
- 恢复后立即检查文件权限：`.env` 和数据库应为 `600`，运行目录应限制服务用户访问。
- GitHub Token、RPC Key、MEV Key 或钱包私钥一旦误提交，应立即撤销/轮换，不能只删除提交文件。
- 迁移完成后核对钱包公钥，不以 wallet_id 或标签单独作为身份依据。
