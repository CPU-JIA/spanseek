# 官网部署

官网源码是 `docs/download` 与 `docs/guide` 中已跟踪的页面和媒体。`prepare.py` 从完整校验的正式发行制品生成公开站点及 Caddy 配置，不依赖桌面应用构建。

主域名为 `https://spanseek.com`，`www.spanseek.com` 永久跳转到主域名。安装包与许可附件连接公开 GitHub Release。Caddy 原生提供 HTTPS、静态文件、视频 Range、压缩及条件缓存。

## 生成

从仓库根目录运行，输出必须是不存在的新目录。测试、预览和部署证据放在 `.local-archive/test-runs/<run>/`。

```powershell
python website/prepare.py --release-directory '<正式制品目录>' --output '<新的公开目录>'
```

生成器检查版本、完整 SHA256SUMS、安装包元数据和哈希，只复制已跟踪的公开页面与媒体，排除文档模板。随后检查相对链接、锚点与模板字段。输出目录旁的 `<目录名>-manifest.json` 记录逐文件大小与哈希，`<目录名>.Caddyfile` 是站点片段。私有配置、凭据和部署记录不进入公开目录。

## 服务器布局

香港服务器现有 Caddy 运行在 Docker 中，宿主机 `/data/caddy/data` 已挂载到容器 `/data`。网站使用这个挂载中的独立子目录，无需重新创建代理容器：

- 宿主机 `/data/caddy/data/spanseek/releases/<发布标识>/`：不可变的公开文件。
- 宿主机 `/data/caddy/data/spanseek/current`：当前版本软链接。
- 容器 `/data/spanseek/current`：站点根目录。

SSH 别名和登录身份只在本机维护。部署前核对服务器、容器与挂载，不能仅凭路径存在判断目标正确。

## 部署与回滚

1. 核对根域 A 与 www CNAME，保留邮箱记录。
2. 备份现有 Caddyfile、current 指向及哈希。上传至新的发布目录，逐文件核对清单。
3. 将生成的两个站点加入原配置，确认没有重复域名，执行 Caddy validate。
4. 原子切换 current，无中断重载。Caddyfile 为文件级 bind mount 时，备份后原位写入以保留 inode；不要替换宿主文件导致容器仍读取旧 inode。
5. 验证公网 HTTPS、根域/www、11 页、404、影片播放/字幕/跳转/Range及下载哈希。

回滚时恢复原 Caddyfile 与 current，验证后重载。保留旧发布目录和备份，回滚无需重新编译桌面程序。

## 与 GitHub 自动同步

服务器的 `spanseek-release-sync.timer` 每 5 分钟运行 `release_sync.py`，读取 `CPU-JIA/spanseek` 的最新正式 Release。草稿、预发布、缺失附件或校验不一致都会拒绝更新，并在服务日志中记录失败。

更新前核对 GitHub 提供的附件 SHA-256、实际下载的 BUILD-INFO 与 SHA256SUMS、安装包名称/版本/大小/哈希和签名状态。这里核对的是发布元数据；安装包本身由正式发行流程验证，不在每次轮询时重复下载。

首页和安装页使用生成器保存于公开目录之外的模板，自动更新版本、下载链接、文件名示例、大小、构建日期、哈希与签名提示。整份站点在新目录中生成后原子切换 current；失败时保留上一份有效站点。浏览器无需调用 GitHub API，禁用 JavaScript 也能得到当前下载信息。公开 `/release.json` 提供本次同步的版本与校验信息。

```powershell
python -m unittest discover -s website -p test_release_sync.py
```

`base` 软链接指向当前官网基线的发布目录，`tools/release_sync.py` 保存执行脚本。部署时安装本目录的同名 systemd service/timer；先手动验证同步，再启用 timer。发布新版官网时先暂停 timer，验证新基线后更新 base，避免自动任务把页面恢复为旧版官网。人工回滚也先停用 timer，核对恢复版本后再恢复自动同步。

元数据通过 GitHub 官方附件 API 下载，避免浏览器下载地址的临时网关故障。公开站点同时提供经过校验的 SHA256SUMS 副本。首页、安装页与同步元数据使用 `Cache-Control: no-cache`，使浏览器重新验证下载信息。网络或 5xx 失败只作一次有界重试；仍失败则留在上一有效版本并记录错误。

参考：[静态站点](https://caddyserver.com/docs/caddyfile/patterns#static-file-server)、[HTTPS](https://caddyserver.com/docs/automatic-https)、[重载](https://caddyserver.com/docs/command-line#caddy-reload)。
