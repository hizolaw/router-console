# Router Console

面向低资源 Linux 路由器的模块化 Web 控制台。Rust / Axum 后端嵌入 Preact 静态前端，发布后无需 Node；系统账户认证模式另有一个只监听本机 Unix socket 的 PAM 验证进程。

**v0.1 是可运行的只读版本。** 支持系统指标、DNS/HTTP 健康探测、订阅更新状态、可选插件和服务入口。尚不包含配置编辑、服务重启或订阅更新操作。

## 快速运行

构建需要 Rust 1.95、C 编译器、Node 22.12+ 和 npm：

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked --release
./target/release/router-console --config config/example.toml
```

打开 **http://127.0.0.1:9080**。示例只启用真实系统指标，不要求安装 Mihomo 或 Sub-Store。Linux 提供 `/proc` 指标；其他系统尚未验证。

前端在编译时嵌入。在同架构、动态加载器和运行库兼容的环境中，运行只需 `target/release/router-console` 和自己的配置文件，不需要源码目录。当前产物是动态链接程序，GitHub 的 Ubuntu 二进制不能直接当成 NixOS 部署包；后续 Nix 打包会处理运行依赖。每次修改前端后应先重新构建前端，再构建 Rust。

可选使用 `just install`、`just build`、`just check`、`just test`、`just run`。

NixOS 开发机可先进入工具环境（此命令使用本机 nixpkgs，尚非锁定的部署构建）：

```sh
nix shell nixpkgs#rustc nixpkgs#cargo nixpkgs#gcc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#nodejs_22 nixpkgs#just
```

## Nix flake 与 NixOS

支持 `x86_64-linux` 和 `aarch64-linux`：

```sh
nix build .#router-console
nix develop
```

Mac mini 的 nix-darwin Linux builder 可构建 ARM64 Linux 包：

```sh
nix build .#packages.aarch64-linux.default \
  --builders @/etc/nix/machines --max-jobs 0
```

在自己的 NixOS flake 中添加并锁定 input：

```nix
inputs.router-console.url = "github:hizolaw/router-console";

# Add this to the target system's modules:
inputs.router-console.nixosModules.default
```

系统模块中配置：

```nix
services.router-console = {
  enable = true;
  listenAddress = "127.0.0.1";
  port = 9080;
  settings.title = "Router Console";
  settings.plugins = [
    { id = "system"; name = "系统资源"; kind = "system"; interval_secs = 5; }
  ];
};
```

默认使用本项目锁定的 nixpkgs 构建程序，不要求旧版宿主 nixpkgs 提供新的 Rust。服务以 DynamicUser 运行，通过 systemd credential 读取令牌，并限制权限与内存。默认首次启动在 `/var/lib/router-console/access.token` 生成 root-only 令牌；重启、切换系统不会覆盖它。管理员可用 `sudo cat /var/lib/router-console/access.token` 读取。

可设置 `tokenFile = "/run/secrets/router-console"` 使用外部提供的运行时令牌；模块不会创建或覆盖外部文件。`environmentFiles` 可为 HTTP 插件提供运行时凭据。不要将秘密写入 `settings`，不要通过 Nix 路径值将秘密复制到 store。

使用路由器现有系统账户登录：

```nix
services.router-console = {
  enable = true;
  authentication = "system";
  allowedUsers = [ "nixos" ];
  listenAddress = "192.0.2.1";
};
```

系统模式通过独立 PAM service 验证指定账户的现有密码和账户状态，不创建密码或解锁账户，不改变 SSH 的认证设置。Web 进程仍是普通用户；仅本机受限 Unix socket 后的 helper 以 root 读取系统认证信息。认证失败统一提示，不向浏览器暴露 PAM 原因。helper 不可用时登录返回错误，已有会话和公开登录页面仍可用。

登录成功生成随机会话，服务端仅保存摘要（最多 128 个），默认 12 小时过期；HttpOnly、SameSite=Strict cookie 使页面刷新后继续登录，退出立即撤销。密码不会写入浏览器存储，标准 username/current-password 表单兼容浏览器密码管理器。会话只在内存，服务重启后需要重新登录。独立配置的 `[auth]` 可设置 `session_ttl_secs` 与 HTTPS 下的 `secure_cookie`；系统模式不会接受旧 Bearer token。

`openFirewall` 默认 false。需要 LAN 访问时设置实际 LAN 监听地址，并按部署的防火墙规则放行；暴露到不可信网络前应配置 HTTPS。`services.router-console.enable = false` 不创建控制台服务，也不停止其他网络服务。

`nix flake check --no-build --all-systems` 检查两种架构和模块启用/禁用；`nix build` 会离线构建前端和 Rust，并运行 Rust 测试。Nix 集成不负责刷机或修改网络角色。

## 配置与插件

```sh
cp config/example.toml config/local.toml
./target/release/router-console --config config/local.toml --check
./target/release/router-console --config config/local.toml
```

修改配置后重启控制台。`config/local.toml` 被 Git 忽略。

| 类型 | 功能 |
| --- | --- |
| `system` | CPU、可用内存、负载、运行时间、接口合计流量与可选温度 |
| `dns` | 直接向 `dns_server` 查询 `dns_name` 的 A 记录 |
| `http` | 检查 `probe_url` 的 HTTP 状态与响应时间 |
| `link` | 提供独立服务入口，不声明健康状态 |
| `subscription` | 从本地脱敏 JSON 状态文件展示最近订阅更新结果 |

所有插件独立配置：

- `enabled = false`：不实例化、不采集、不出现在 API 中。
- `visible = false`：首页隐藏，仍采集，在“全部服务”中展示。
- `interval_secs`：5–3600 秒；默认 15 秒。失败后逐步退避，最高为 300 秒或原周期，两者取较大值。
- `url`：浏览器打开的管理页面；`probe_url`：后端访问的健康接口。远程访问时，两者可能需要使用不同地址，浏览器的 `localhost` 指浏览器所在设备。
- `url_env`：可选的运行时环境变量名，指定后优先于 `url`。变量值通过已认证的状态 API 返回浏览器作为服务入口；缺失或非法时隐藏入口，不影响健康探测。值只允许不含用户名/密码的 HTTP(S) URL，最长 8192 字节。可由 `environmentFiles` 提供含自动配置参数的入口，避免把令牌写进 Nix store；入口不会写入浏览器持久存储。仅向可信管理用户提供此入口，打开目标页面后 URL 参数仍可能进入浏览器历史和目标服务日志。修改环境文件后重启控制台。
- `token_env`：HTTP 探测的 Bearer 凭据环境变量名；凭据不返回浏览器。
- `status_file`：`subscription` 插件必须指定的本地 JSON 文件路径。最多读取 64 KiB，支持 `lastAttempt`、`lastSuccess`、`error`、`count`、`attemptStatus` 字段。时间字段为字符串，节点数为非负整数；最近尝试 `success` 显示正常，`failed` 显示降级，`skipped` 或状态缺失显示未知。文件读取或解析失败显示通用错误，不暴露路径。文件必须由可信更新器生成，`error` 必须已经脱敏；不要指向包含订阅凭据的文件。NixOS 使用 DynamicUser 时，通过专用组和目录权限授予只读访问。

最多 32 个插件配置项，探测并发最多 4 个，单次探测最多 4 秒。多浏览器共享缓存。页面可见时每 5 秒刷新，过期结果显示“未知”。

HTTP 2xx 只证明接口可达，不等于代理出口正常；不跟随重定向，不使用环境代理。DNS 探测只覆盖 UDP A 查询，截断响应会报错，不支持 TCP 回退或 DNSSEC 验证。接口流量为所有非 lo 接口合计，桥接/TUN 可能重复计数，不能当成 WAN 流量。

## 局域网访问与认证

默认仅监听 loopback，可不设置令牌。监听 LAN 地址或 `0.0.0.0` 时必须提供至少 24 字节的访问令牌，否则拒绝启动。

创建运行时令牌文件（不要提交）：

```sh
mkdir -p ~/.config/router-console
(umask 077; openssl rand -hex 32 > ~/.config/router-console/access.token)
./target/release/router-console --config config/local.toml \
  --token-file ~/.config/router-console/access.token
```

在配置中设置实际监听地址。也可以通过 `ROUTER_CONSOLE_TOKEN` 提供令牌。浏览器会要求输入令牌，只保存在当前页面内存中，刷新后需重新输入。API 使用 `Authorization: Bearer ...`。

API 仅提供只读 `GET /api/status`，不支持跨域访问。访问令牌不会给 HTTP 传输加密；跨越不可信网络时使用 HTTPS 反向代理或 SSH 隧道，后端保持 loopback 监听。控制台默认无需 root，也不需要 systemd 管理权限。

## 开发与验证

```sh
npm run build --prefix web
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

浏览器测试（先构建 release）：

```sh
cd web
npx playwright install chromium
npm run test:e2e
```

NixOS 可使用系统 Chromium：

```sh
nix shell nixpkgs#chromium -c sh -c 'PLAYWRIGHT_CHROMIUM_EXECUTABLE="$(command -v chromium)" npm run test:e2e --prefix web'
```

浏览器测试使用专用 loopback 端口 19080 和公开的测试令牌，仅用于测试。截图保存在 `artifacts/`，不提交。

GitHub Actions 分别在 x86_64 和 ARM64 Linux 上构建和运行 Rust 测试，x86_64 额外运行浏览器测试，提供对应二进制 artifact。普通 Linux 构建成功不等于 R4S 实机运行验证。

## 项目边界

控制台停止不应影响基础网络。DNS、Mihomo、Sub-Store 和订阅更新由部署侧分别管理，本程序不负责路由、DNS 配置或网络故障恢复。没有创建或修改这些服务的运行配置。

更多接口边界和后续计划见 [架构设计](docs/architecture.md)，本地测试和资源测量见 [验证记录](docs/verification.md)。

参考：[Glance](https://github.com/glanceapp/glance)、[Homepage](https://github.com/gethomepage/homepage)、[LuCI](https://github.com/openwrt/luci)、[Zashboard](https://github.com/Zephyruso/zashboard)、[Sub-Store](https://github.com/sub-store-org/Sub-Store)。本项目独立实现控制台，未复制这些项目的代码或资源。
