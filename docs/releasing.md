# 构建、缓存与版本发布

**[`1.0.2`](https://github.com/CPU-JIA/spanseek/releases/tag/v1.0.2) 已于 2026-11-06 公开发布**，tag `v1.0.2` 固定到 `07b9f03df5a9f0beb974be43ffbd5a1f08a7c61b`。`v1.0.1` 继续保持冻结；后续按 `1.x` 管理，不回退已安装版本号。

本次 Windows 构建 [42196265439](https://github.com/CPU-JIA/spanseek/actions/runs/42196265439) 通过全部工程检查，tag 工作流 [42212880582](https://github.com/CPU-JIA/spanseek/actions/runs/42212880582) 复用同一提交的制品。实际安装包已完成隔离安装、13 项核心界面检查、中英文扫描 PDF 与 Unicode 路径 OCR、正常退出和隔离卸载；14 个公开附件均已匿名下载并核对 SHA-256。隔离卸载使用 `/S /UPDATE` 保留共用 AppID 状态，不将其写成普通交互卸载验收。安装包仍为 `NotSigned`；网站已从该制品生成并验证，域名网站未在本次发布中部署。

## Actions 行为

构建工作流：[windows-checks.yml](../.github/workflows/windows-checks.yml)；tag 草稿工作流：[release-draft.yml](../.github/workflows/release-draft.yml)；公共环境准备：[windows-setup](../.github/actions/windows-setup/action.yml)。

| 触发                                    | 执行内容                                                                            | 结果位置                             |
| --------------------------------------- | ----------------------------------------------------------------------------------- | ------------------------------------ |
| 仅官网/已列明用户文档的 PR 或 main push | 网站链接、字幕时间、视频完整解码、JavaScript 语法、版本一致性及源码输入检查         | Actions 运行日志；不重复打桌面安装包 |
| 其他 Pull request                       | 上述检查 + fmt、Rust 测试、clippy、DTO 漂移、typecheck、串行桌面测试、前端构建      | Actions 运行日志                     |
| 其他 main push                          | 上述完整检查 + Windows NSIS 安装包及清单                                            | 本次运行的 Artifacts，保留 14 天     |
| 手工 workflow_dispatch                  | 与 main 相同                                                                        | Actions Artifacts                    |
| 推送 `v*` tag                           | 核对 tag/版本/提交；复用同一提交的成功 main 构建，逐文件校验后创建 Release **草稿** | Releases 草稿                        |

构建使用 `windows-2025`、Rust 1.98.0、Node 24.19.0 和 pnpm 10.25.0。所有外部 Action 固定到提交 SHA。普通检查只有 `contents: read`；仅 tag 的独立草稿上传 job 有 `contents: write`，使用 GitHub 提供的临时 token，无须保存个人访问令牌。

`scripts/ci-scope.py` 根据完整 Git 差异判断范围，未知文件、缺失历史、首次 push、手工运行都执行完整检查；移动文件同时检查删除和新增路径。工作流始终运行 `windows` 必需检查，不在触发层按路径跳过，避免分支保护永久等待。仅官网改动时仍验证真实媒体和本地链接；安装器模板中的两个下载路径由正式制品准备脚本核验。视频解码使用固定版本、固定 Windows wheel SHA-256 的 FFmpeg 工具包，不需要编译桌面程序。

流程不会自动公开 Release，不覆盖已发布的同名版本。首次远程运行成功前，只能称工作流已准备并经过本地静态/脚本验证；不能称 GitHub 构建已通过。

若要给一个仅文档或官网改动的提交创建新版本 tag，先在 main 上手工运行本工作流，取得该准确提交的完整安装包与清单，再按后述步骤发布。快速网站检查的成功记录不能代替安装包构建。

## 缓存复用

| 缓存         | 键与失效条件                                                                                      | 内容                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| pnpm         | 系统/架构、pnpm 版本、`pnpm-lock.yaml`；锁变化可恢复同版本旧 store                                | 包 store；安装仍执行 `--frozen-lockfile`                                                   |
| Cargo 包     | `Cargo.lock`；命中旧键后先用 `cargo fetch --locked` 补全当前 Windows 依赖                         | Cargo registry/git；只在补全成功后由非 PR 运行保存                                         |
| Rust 检查    | 工具链/平台前缀、清单/构建配置指纹、提交 SHA、run ID、attempt；逐级恢复同提交/同配置/同工具链快照 | `target/debug` 的依赖和工作区编译结果                                                      |
| Rust release | 同上，独立的 release 命名空间                                                                     | `target/release`，仅需要安装包时恢复                                                       |
| 原生资源     | 系统/架构、下载/构建脚本、vcpkg baseline/triplet、来源清单哈希                                    | PDFium、OCR/语言包、ONNX Runtime 静态库、vcpkg 下载及 ABI 二进制缓存、protoc、对应源码归档 |

完整 main 构建生成 release 产物，后续 tag 直接复用同提交制品；PR 只读取 Rust 缓存。GitHub 缓存受分支作用域、容量和回收策略限制，命中率不作保证。每次都运行必要检查和资源校验，缓存命中不代表构建或测试已经成功。

pnpm 在依赖安装完成后保存缓存，原生资源在校验完成后保存。GitHub 缓存不能覆盖已有键；编译缓存每轮使用新快照键，检查完成后保存 debug、安装包构建成功后保存 release，避免“精确命中旧缓存→新编译结果永远不回存”。安装包后续清单校验失败不会抹掉已经成功编译并保存的缓存，但仍阻止制品交付。首轮新缓存缺失时只读复用原来的依赖缓存，此后恢复完整新快照。

缓存保留不同 profile/features 的编译产物；新版本仍须重编译受改动影响的 crate。热构建约 15 分钟是优化目标，须用真实连续运行的阶段耗时验证，不能仅凭缓存命中宣布达标。首次完整缓存填充、工具链/依赖升级可能显著更久；滚动快照也会受仓库容量与自动回收影响。

CI 的 `ORT_CACHE_DIR` 固定到 `.local-archive/dependencies/ort`，与 Cargo 编译缓存一起恢复。`fetch-onnxruntime.py` 在编译前验证压缩包及两个原生文件的 SHA-256，缺失时从固定来源恢复；若 Cargo 的 `ort-sys` 链接记录指向其他目录，只清理并重建该 crate。已验证的原生下载在 worker 编译前保存，后续编译失败不会使下载缓存丢失。

不缓存知识库、AppData、凭据、模型权重或整个 `.local-archive`。安装包作为 Artifact/Release 附件交付，不提交到 Git。

## 版本和 tag

保持以下版本一致：

- `Cargo.toml` 的 `[workspace.package].version`，及 Cargo 更新后的 `Cargo.lock`。
- `apps/desktop/package.json`。
- `apps/desktop/src-tauri/tauri.conf.json`。
- `CHANGELOG.md` 和用户文档中的版本说明。

`packages/contracts` 是私有工作区包，自己的包版本不代表桌面产品版本。版本检查命令：

```powershell
node scripts/check-version.mjs --tag v1.0.2
```

每次先完成代码审阅与检查，再按已授权范围推送。以下以 1.0.2 为例；这些是发布步骤，不由本地整理脚本自动执行：

```powershell
git add .
git commit -m "Prepare spanseek 1.0.2 release"
git push -u origin main
# main 的 Actions 构建成功并核对产物后：
git tag -a v1.0.2 -m "spanseek 1.0.2"
git push origin v1.0.2
```

后续每个新版本使用新 tag；不要强推或移动已发布 tag。tag 使用独立工作流，避免跳过的构建任务污染 main 的必需检查状态。tag 不重复编译相同源码：它只接受本仓库、同一 workflow、main 分支、完全相同提交 SHA 的成功构建，拒绝 PR、失败运行、过期或重复附件。下载后再校验版本、commit、全部文件哈希、安装包大小/签名状态与源码清单。找不到合格构建时失败，须先完成同提交 main 构建再重跑 tag。tag 的草稿发布记录独立保留，不用不同提交的旧成功结果代签。

## 制品与公开前检查

官网已于 2026-11-08 另行部署到 [spanseek.com](https://spanseek.com)，由香港服务器 Caddy 提供服务。下载信息通过独立定时任务跟随 GitHub 最新正式 Release；部署、校验与回滚说明见 [website/README.md](../website/README.md)。

`scripts/package-release.ps1` 汇总实际安装包、`SHA256SUMS.txt`、`BUILD-INFO.json`、依赖清单、第三方声明和版本记录。清单记录实际 commit、大小、哈希和 Authenticode 状态。当前没有代码签名证书配置，输出应如实为 `NotSigned`。

安装包和 Release 附件均须包含完整 `THIRD-PARTY-LICENSES.txt`、组件文件哈希清单、`THIRD-PARTY-SOURCES.zip` 与源码说明。OCR 从固定源码构建，并通过真实中英文 PNG/扫描 PDF/中文路径测试；PDFium 与 ONNX Runtime 的上游第三方声明也纳入许可材料。`build-license-bundle.py` 的缺失项必须为 0，元数据清单不能替代许可原文。

未签名不会阻止 GitHub 发布，但 Windows SmartScreen 或 Smart App Control 可能提示或阻止安装。发行说明必须如实标注；不要求用户关闭系统安全保护。

下载该 tag 的安装包，完成隔离安装、启动、关键内容保存/重开和卸载验证，再检查 Release 草稿与校验值，最后按授权公开。修改同一草稿可重跑该 tag 的流程；已有公开 Release 会被拒绝覆盖。

GitHub 的按 tag 查询接口可能对草稿返回 404；当前工作流会继续分页查询含草稿的发布列表，避免重跑时重复创建。已冻结的 `v1.0.1` tag 保留早期工作流，不能通过移动 tag 修补：如需重建它，先核对新 Artifacts 的版本、校验值和 `BUILD-INFO.json` 中的 commit 与 tag 指向一致，再用 `gh release upload v1.0.1 <制品文件列表> --clobber` 更新仍为草稿的附件。执行前必须确认该 Release 尚未公开；当前主分支的修复适用于后续 tag。

官网源码在 `docs/download`。安装页的大小、构建日期和哈希是模板字段，不绑定某一次本机构建。下载并核验 Actions 的全部制品后，用下面的命令生成一个新的本地预览目录：

```powershell
pwsh -NoProfile -File scripts/prepare-download-site.ps1 -ReleaseDirectory '<已解压的制品目录>' -Output '.local-archive/test-runs/download-site-<日期>/site'
```

脚本核对 `BUILD-INFO.json`、安装包和全部 `SHA256SUMS.txt` 条目，仅复制 Git 跟踪的官网、网页指南与校验后的制品，生成 `site/docs/download/install.html`。本地开发记录不会进入输出；既有目录不会被覆盖。通过 HTTP 服务预览输出目录，并核对下载与哈希复制后，再按授权部署整个输出目录。官网及下载目录不会在 Actions 或此脚本中自动部署。

正式域名网站使用 [website 部署工具](../website/README.md)：从同一正式制品校验后生成域名根目录版本与 Caddy 站点配置，安装包与许可附件连接已有 GitHub Release。香港服务器现有 Caddy 提供 HTTPS、静态文件及视频 Range，`www.spanseek.com` 跳转到 `spanseek.com`。首次上线前备份原配置，验证新配置和公开文件哈希，再无中断重载；该流程不编译或重新发布桌面应用。

参考：[GitHub 缓存机制](https://docs.github.com/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows)、[Rust cache](https://github.com/Swatinem/rust-cache)、[Windows runner 软件清单](https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md)。
