# 开发指南

跨索采用 Tauri 2、React/TypeScript 和独立 Rust 核心。当前版本为 1.0.1，支持 Windows x64；应用内部 crate 和包名保留 `pkms`，安装身份保留 `com.pkms.desktop`。

## 环境

- Windows 10/11 x64、PowerShell 7、Microsoft Edge WebView2。
- Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”及 Windows SDK。
- Rust/Cargo 1.98.0，带 rustfmt 和 clippy。
- Node 24.19.0、pnpm 10.25.0。
- 7-Zip：用于提取固定版本的 OCR 运行库；不运行 OCR 安装程序、不修改系统安装。GitHub `windows-2025` 已包含它。
- Python 3.11 或更高版本：生成发布清单；应用运行不依赖 Python。

```powershell
rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
rustup override set 1.98.0
npm install --global pnpm@10.25.0
pnpm install --frozen-lockfile
pnpm desktop
```

`pnpm desktop` 会准备富格式导出资源、PDFium、OCR、protoc 和原生 worker，然后启动 Vite 与 Tauri。首次需要下载依赖，后续复用本地缓存。模型需在应用内按需配置；构建流程不下载生成模型。

## 目录

| 路径                                   | 内容                                                        |
| -------------------------------------- | ----------------------------------------------------------- |
| `apps/desktop`                         | React 工作台与 Tauri 桌面壳                                 |
| `apps/mobile-capture`                  | 配套移动捕获、录音与续读网页                                |
| `crates/pkms-core`                     | SQLite、正文、组织、修订、检索与备份等核心能力              |
| `crates/pkms-app`                      | 应用服务、授权、任务和 IPC 协调                             |
| `crates/pkms-worker`、`pkms-inference` | 解析与推理进程                                              |
| `crates/pkms-sync*`、`pkms-external`   | 同步协议、服务及外部接口                                    |
| `packages/contracts`                   | 由 Rust 定义生成的 TypeScript DTO                           |
| `scripts`                              | 资源准备、构建、版本校验与发布清单                          |
| `third-party`                          | 必需的许可、字体声明与本地修补的依赖源码                    |
| `docs`                                 | 用户、接入、开发与发布文档；`download` 为官网源码和自有媒体 |

`.local-archive`、依赖缓存、测试库、密钥、安装包和本机开发记录不属于源码分发。`crates/pkms-core/resources/rich-export.zip` 是核心编译时使用的离线资源，保留在源码树；修改相关渲染源码后通过脚本重建。

## 本地缓存与原生资源

Cargo 默认写入 `target/`。需要换盘时可复制 `.cargo/config.example.toml` 为 `.cargo/config.toml`，或在当前 PowerShell 设置 `$env:CARGO_TARGET_DIR`。本机绝对路径不提交。

资源脚本固定下载版本并检查 SHA-256；命中缓存也要核对。OCR 的运行库、语言数据及许可清单以 `third-party/tesseract/runtime-manifest.json` 为准。自定义已安装的 OCR 来源可用：

```powershell
pwsh -NoProfile -File scripts/prepare-tesseract.ps1 -SourceRoot 'D:/tools/tesseract'
```

版本或哈希不符会失败，不会换成其他版本或跳过 OCR。下载位置分别在 `.local-archive/dependencies/` 和 `.local-archive/tooling/`；这些目录可以从固定来源重新生成。

## 检查与构建

在新环境先准备编译和测试需要的原生资源：

```powershell
python scripts/check-source-inputs.py
pwsh -NoProfile -File scripts/prepare-desktop-resources.ps1
$env:PROTOC = Join-Path (Get-Location) '.local-archive/tooling/protoc-36.2/bin/protoc.exe'
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm contracts:generate
pnpm typecheck
pnpm --filter @pkms/desktop exec node --experimental-strip-types --test --test-concurrency=1 tests/*.test.mjs
pnpm desktop:build
```

共享 DTO 不手工修改；生成后检查 `packages/contracts/src/generated` 的差异并一并提交。桌面测试串行运行，避免多个 WebView/渲染测试争抢资源。测试数据使用新建的隔离目录，勿以日常知识库作为写入夹具。

源码预检核对 Rust 字面量 `include_str!` / `include_bytes!` 引用的文件是否已提交；动态生成的原生资源仍由准备脚本验证。合成问答/检索数据位于 `crates/pkms-app/tests/fixtures/quality`，不依赖本地实验目录。

浏览器上传测试自动查找标准安装位置的 Edge/Chrome，也可通过 `CHROME_BIN` 指定可执行文件。标为 ignored 的真实 E5 模型测试需要另外准备模型并设置 `PKMS_E5_MODEL_DIR`；它们不因普通测试通过就算作已经执行。

安装包位于 Cargo 目标目录的 `release/bundle/nsis/`。它需要独立验证安装、启动、保存/重开、卸载及真实数据保护；工程测试通过不能代替这些系统行为。

官网用户文档由对应 Markdown 和 `docs/guide/doc-template.html` 生成。安装 Pandoc 后运行 `pwsh -NoProfile -File scripts/build-user-docs.ps1`，再用 `pnpm exec prettier --write 'docs/guide/*.html'` 格式化；用户手册、HTML 与版本记录一起提交。

## 数据与接口约束

`pkms-core` 不依赖 Tauri/UI；前端不直接写数据库。保存、恢复和 AI 提案核对基准修订；正文、修订和必要回执原子提交。重型解析/推理交给 worker，原件不可变。未知未来 schema 必须拒绝，历史恢复不降成纯文本。外部接入见[接口契约](external-api.md)。

## 许可

仓库沿用 [LICENSE](../LICENSE) 的保留所有权利条款；上传源码不等于改为开源许可。第三方依赖继续使用各自许可，索引见[第三方声明](../THIRD-PARTY-NOTICES.md)。
