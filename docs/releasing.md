# 构建、缓存与版本发布

当前首个 GitHub tag 使用 **`v1.0.1`**，与已经交付的桌面版本一致。后续按 `1.x` 管理；不为了新建仓库把已安装版本回退成 `0.1.0` 或 `1.0.0`。

## Actions 行为

工作流：[windows-checks.yml](../.github/workflows/windows-checks.yml)；公共环境准备：[windows-setup](../.github/actions/windows-setup/action.yml)。

| 触发                   | 执行内容                                                                        | 结果位置                         |
| ---------------------- | ------------------------------------------------------------------------------- | -------------------------------- |
| Pull request           | 版本一致性、fmt、Rust 测试、clippy、DTO 漂移、typecheck、串行桌面测试、前端构建 | Actions 运行日志                 |
| 推送 main              | 上述检查 + Windows NSIS 安装包及清单                                            | 本次运行的 Artifacts，保留 14 天 |
| 手工 workflow_dispatch | 与 main 相同                                                                    | Actions Artifacts                |
| 推送 `v*` tag          | 检查 tag 与源码版本一致；上述检查和构建；创建或更新同 tag 的 Release **草稿**   | Artifacts + Releases 草稿        |

构建使用 `windows-2025`、Rust 1.98.0、Node 24.19.0 和 pnpm 10.25.0。所有外部 Action 固定到提交 SHA。普通检查只有 `contents: read`；仅 tag 的独立草稿上传 job 有 `contents: write`，使用 GitHub 提供的临时 token，无须保存个人访问令牌。

流程不会自动公开 Release，不覆盖已发布的同名版本。首次远程运行成功前，只能称工作流已准备并经过本地静态/脚本验证；不能称 GitHub 构建已通过。

## 缓存复用

| 缓存     | 键与失效条件                                                           | 内容                                                 |
| -------- | ---------------------------------------------------------------------- | ---------------------------------------------------- |
| pnpm     | 系统/架构、pnpm 版本、`pnpm-lock.yaml`；锁变化可恢复同版本旧 store     | 包 store；安装仍执行 `--frozen-lockfile`             |
| Rust     | 固定 shared key，加 Rust 工具链、Cargo 清单/锁文件和编译环境的自动哈希 | Cargo registry/git 和 `target` 中依赖/工作区构建结果 |
| 原生资源 | 系统/架构、下载/准备脚本及 Tesseract 固定清单哈希                      | PDFium、OCR 运行库与语言包、protoc 下载              |

main 构建也生成 release 产物，使后续 tag 可以复用默认分支的编译缓存；PR 不保存 Rust 缓存。GitHub 缓存受分支作用域、容量和回收策略限制，命中率不作保证。每次都运行必要检查和资源校验，缓存命中不代表构建或测试已经成功。

不缓存知识库、AppData、凭据、模型权重或整个 `.local-archive`。安装包作为 Artifact/Release 附件交付，不提交到 Git。

## 版本和 tag

保持以下版本一致：

- `Cargo.toml` 的 `[workspace.package].version`，及 Cargo 更新后的 `Cargo.lock`。
- `apps/desktop/package.json`。
- `apps/desktop/src-tauri/tauri.conf.json`。
- `CHANGELOG.md` 和用户文档中的版本说明。

`packages/contracts` 是私有工作区包，自己的包版本不代表桌面产品版本。版本检查命令：

```powershell
node scripts/check-version.mjs --tag v1.0.1
```

新仓库首传当前完整版本，不迁入旧项目提交历史。在代码审阅、检查通过且获得推送授权后，可使用以下顺序；这些命令是发布步骤，不会由整理脚本自动执行：

```powershell
git add .
git commit -m "Initial spanseek source import"
git push -u origin main
# main 的 Actions 构建成功并核对产物后：
git tag -a v1.0.1 -m "spanseek 1.0.1"
git push origin v1.0.1
```

后续每个新版本使用新 tag；不要强推或移动已发布 tag。main 和 tag 的构建各有自己的运行记录，不把前一次构建状态代替当前 tag 的结果。

## 制品与公开前检查

`scripts/package-release.ps1` 汇总实际安装包、`SHA256SUMS.txt`、`BUILD-INFO.json`、依赖清单、第三方声明和版本记录。清单记录实际 commit、大小、哈希和 Authenticode 状态。当前没有代码签名证书配置，输出应如实为 `NotSigned`。

下载该 tag 的安装包，完成隔离安装、启动、关键内容保存/重开和卸载验证，再检查 Release 草稿与校验值，最后按授权公开。修改同一草稿可重跑该 tag 的流程；已有公开 Release 会被拒绝覆盖。

官网源码在 `docs/download`。实际安装包由独立部署步骤放入 `docs/download/downloads/`，并按同一份包更新安装页大小、日期和哈希；不要把本机构建的校验值套给 Actions 构建。官网及下载目录不会在本工作流中自动部署。

参考：[GitHub 缓存机制](https://docs.github.com/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows)、[Rust cache](https://github.com/Swatinem/rust-cache)、[Windows runner 软件清单](https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md)。
