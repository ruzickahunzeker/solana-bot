# TS-02-C6 借用交易输入与纯地址解析边界

日期：2026-11-13。状态：源码完成，新增 9 项离线回归，原 5 项参数测试迁移至 Application；全套 325 → 334，通过。未升级 SDK/vendor/依赖、构建 release、部署/重启、读取生产钱包/数据库或发送交易。

## 已实施

- `application/trade_input.rs::TradeInput` 是无 serde 的借用输入，只包含普通校验读取的 22 个字段，不复制钱包和来源元数据，不解析地址或应用默认值。
- API `TradeRequest::validation_input` 显式映射；字符串保持原借用、空白和字节，Option 保持三态，没有新增 wire 字段或别名。
- `application/ports/address_parser::TradeAddressParser` 只暴露字符串到项目 Address 的纯语法解析，不含 RPC、钱包、签名或协议校验。
- `adapters/sdk/address_parser::SdkAddressParser` 仍调用原 `Pubkey::from_str`；不 trim、不替换算法、不在 adapter 做 SOL/WSOL 别名转换，原解析错误仍作为 anyhow 原因链保留。
- `application/trade_validation::TradeValidator` 拥有全部普通金额/模式/MEV/协议输入规则，依赖项目输入、5 字段 settings、Registry 与解析 port；不导入 API DTO、Config、SDK 或具体 adapter。
- `validation.rs` 保留旧公开路径，映射 Config/DTO 并注入原 parser；Warmup 仍在兼容边界将项目地址转换为原 Pubkey 结果。未把 Warmup 套入普通金额或模式政策。

地址语法通过不等于池身份、Quote 资产、余额、扩展、报价、路由时效或签名授权通过；本轮不开放任何交易能力。

## 保持的顺序与兼容行为

普通校验仍按 key → 金额字段互斥 → Mint → Pool ID → Quote/别名/Quote 政策 → 滑点 → 小费 → MEV → hop 地址/Via-SOL 政策 → 买入量或卖出比例 → 执行方式 → Pool ID 政策。

空白 Mint/Pool ID/hop 地址保持 `is required`；空 Quote 保持 `quote_mint must be a valid pubkey`。不会因为注入 parser 而提前解析、改变错误文本或预先校验未使用的默认值。

历史 replay 是独立入口，只检查旧金额字段形状、Quote 解析和旧 aggregator 分支选择；不读取当前 settings，不补校验历史 key/Mint/Pool/hop，不应用当前协议许可、滑点/小费或 Via-SOL 开关。旧 request hash、请求 JSON、schema 和 replay 指纹不改。

## 验证范围

原 5 项参数测试迁入 `application/trade_validation/tests.rs`，使用项目输入与只读 fixture parser，无 DTO/Config/SDK。新增 9 项：

1. DTO 到视图的全部 22 个校验字段、字符串借用指针与缺省 None 映射。
2. key/金额形状错误先于 parser，parser 调用数为零。
3. 原始字符串不 trim，Mint/Pool/Quote/hop 的错误顺序及错误原因链。
4. hop 解析晚于 Quote/滑点/小费/MEV，逐阶段验证调用轨迹。
5. replay 不解析无关地址，不施加当前默认值与准入规则。
6. replay 金额形状先于 Quote，旧协议政策与当前校验保持独立。
7. 原始 Quote `u64::MAX`、SOL alias 和 simulate 三态通过纯项目入口完整保留。
8. SDK parser 的 4 个常量与 32 组合成地址逐字节一致；parser 本身不合并 SOL/WSOL。
9. 空白、前后空格、非法字符、长度和 Unicode 输入保持 SDK 语法错误。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet application::trade_validation
cargo test --locked --offline --quiet
git diff --check
```

结果：精准 11 项、全套 334 项通过，0 failed，无编译警告。原 normalization/hash、入库/record 字节、HTTP/幂等、loader 和故障恢复期待值未修改。Cargo.toml/Cargo.lock/现有 release SHA-256 与上一批一致；vendor 与原迁移 diff 为空。

## 后续边界

- TS-02-C7：拒绝订单仍在 HTTP 使用具体 TradeStore 与 DTO mirror；先冻结现有记录/时钟/日志/持久化惰性及失败响应语义，再接入项目 admission/journal port。不能因为无效请求而占用正常交易幂等键。
- Warmup 内部模型、普通发送前签名账本、VerifiedRoute、完整协议证据及 SDK provenance 仍独立推进。
- 本轮只证明输入政策与 parser 的离线隔离，不代表编译后的 release 已部署或任何真实 RPC/交易已验收。
