# @pkms/contracts

Rust 模型的 TypeScript 导出。类型源来自 `pkms-core`（文档、修订、图片与提交）和 `pkms-app`（本机状态、授权与启动操作）；`scripts/generate-contracts.mjs` 顺序调用两个 crate 的导出器，并统一生成文本的行尾空白。

在项目根目录运行：

```powershell
pnpm contracts:generate
pnpm typecheck
```

`src/generated` 不手工维护。当前包含核心请求/回执、分页摘要、本机状态、Tauri 请求/响应 envelope、结构化 IPC 错误与刷新事件。Rust 负责运行期校验，客户端通过 `apps/desktop/src/api.ts` 调用窄 IPC，不直接写库。
