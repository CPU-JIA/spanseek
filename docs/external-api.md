# spanseek 外部接口契约（v1）

适用于 spanseek 1.0.1。适用于 MCP stdio 与本地 HTTP 两个适配器；二者复用同一份请求、响应、错误码、作用域校验与审计。

## 协议标识

- `protocol`: `spanseek.external/v1`
- `protocol_version`: `1`
- 请求信封：

```json
{
  "protocol": "spanseek.external/v1",
  "request_id": "req-20261020-001",
  "command": {
    "method": "search",
    "params": {
      "query": "知识管理",
      "mode": "lexical",
      "scope": {},
      "limit": 10,
      "minimum_cosine": null
    }
  }
}
```

- 响应信封：

```json
{
  "protocol": "spanseek.external/v1",
  "request_id": "req-20261020-001",
  "ok": true,
  "data": { "items": [] },
  "error": null
}
```

## 方法与作用域

| 方法           | 作用域    | 语义                                                                 |
| -------------- | --------- | -------------------------------------------------------------------- |
| `capabilities` | `read`    | 返回协议、方法、作用域、默认只读与写入模式                           |
| `search`       | `search`  | 检索笔记/来源证据；不写库                                            |
| `read_note`    | `read`    | 读取指定笔记或修订；不写库                                           |
| `list_notes`   | `read`    | 有界列出笔记摘要；不返回完整正文                                     |
| `propose_note` | `propose` | 只创建待人工确认的提案，不直接改正文                                 |
| `audit_page`   | `audit`   | 查询外部接口审计；可按请求 ID、主体 ID、对象 ID 筛选；不含正文与令牌 |

## 错误码

`unsupported_protocol`、`invalid_request`、`unauthorized`、`forbidden`、`not_found`、`conflict`、`unavailable`、`write_proposal_only`、`internal`。

错误响应不包含私库正文、令牌、完整路径或底层 SQL。`forbidden` 必须带 `required_scope`，便于客户端区分缺少作用域与令牌无效。

## 兼容策略

1. `v1` 内只允许新增可选字段、可选方法和向后兼容的行为修正；不得复用或改变已有字段语义。
2. 删除字段、改变必填性、改变错误含义或改变写入边界，必须发布 `spanseek.external/v2`，并在一段时间内并行提供 v1。
3. 未知协议版本、未知方法、未知命令字段或未知参数字段必须明确拒绝，不静默降级。
4. 客户端必须忽略响应中的未知可选字段；服务端不得依赖客户端未声明的扩展。
5. 任何新增方法必须同时更新 MCP 工具描述、HTTP 路由、作用域和负向测试。

## 默认安全边界

- 默认只读；写入只生成提案，不直接写正文。允许的只读调用会追加不含正文/查询词/令牌的审计元数据；知识对象、修订和提案不变。
- 本地 HTTP 仅绑定 `127.0.0.1`，Bearer 令牌只保存哈希。
- 令牌明文只在创建时显示一次；日志、审计和响应中不得出现明文。
- 真实库 `我的知识库` 的测试只做只读核对；所有写入实验使用隔离现场。

实现锚点：`crates/pkms-core/src/external_contract.rs`。

## 审计

外部请求写入 schema 27 的 `external_audit_events`。记录字段：

- 请求 ID、时间、传输、主体（HTTP 令牌 ID 或 `mcp-local`）、方法；
- 所需作用域、实际授予作用域、`allowed` / `denied` / `failed`、错误码；
- 主体对象 ID（仅笔记 ID 等稳定标识）和耗时。

审计表不存查询词、正文、摘录、文件路径、Bearer 明文或密钥。认证失败也会记录为匿名主体。令牌撤销由运行中的 HTTP 服务逐请求重读令牌文件，无需重启。

## 本地外部工具

```powershell
spanseek-external serve-mcp --workspace <隔离库> [--enable-proposals] [--enable-audit]
spanseek-external serve-http --workspace <隔离库> --token-file <文件> --port 8787 [--enable-proposals] [--enable-audit]
spanseek-external token-create --token-file <文件> --label agent --scopes read,search
spanseek-external token-list --token-file <文件>
spanseek-external token-revoke --token-file <文件> --id <token_id>
spanseek-external proposal-get --workspace <隔离库> --id <proposal_id>
spanseek-external proposal-apply --workspace <隔离库> --id <proposal_id> --confirm
spanseek-external proposal-reject --workspace <隔离库> --id <proposal_id> --confirm
spanseek-external draft-report --workspace <隔离库> --query <问题> --title <标题> --mode evidence --output <report.json>
spanseek-external verify-report --workspace <隔离库> --report <report.json>
spanseek-external export-reports --workspace <隔离库> --input <report.json> --formats md,docx,pdf --output-dir <目录>
```

提案读取/应用/拒绝是本地人工操作，不暴露为 MCP/HTTP 写工具。`proposal-apply` 与 `proposal-reject` 必须显式 `--confirm`。更多能力边界见 [external-capabilities.md](external-capabilities.md)。
