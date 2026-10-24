# v0.1 验证记录

日期：2026-10-24。此记录对应首个 Rust 实现及最后的界面微调；不是 R4S 实机验收。

## 本地检查

- x86_64 Linux，Rust 1.95.0，Node 22.22.3。
- 前端 TypeScript 检查、Vite production 构建通过。
- Rust 格式、Clippy（warnings 视为错误）、10 项测试通过。
- release 构建成功，静态资源已嵌入；从源码目录外启动仍能提供页面。
- Chromium 149 的两项浏览器测试通过，覆盖正确/错误令牌、退出、隐藏与禁用、故障卡片、过期状态、桌面与 390px 手机布局，无页面脚本或 CSP 错误。

Rust 测试覆盖：配置约束、URL 协议与凭据限制、过期状态、探测超时释放许可、禁用/隐藏、HTTP 错误与重定向、DNS 响应匹配、CPU/内存/接口数据解析、认证和 Host 边界。

## 资源冒烟测量

测试环境：8 个逻辑 CPU 的 x86_64 Linux 开发机，release 二进制，非 R4S。

启用 3 个插件：真实系统指标、loopback HTTP、loopback DNS 测试响应器。采集周期均为 5 秒，一个客户端每 5 秒读取状态 API。持续 30.03 秒，每秒采样一次进程 RSS。

| 项目 | 结果 |
| --- | --- |
| RSS 最小 / 最大 | 6.64 / 6.69 MiB |
| release 文件大小 | 约 4.9 MiB |
| 首页 JS + CSS 原始体积 | 约 31.9 kB |
| 首页 JS + CSS gzip 体积 | 约 12.3 kB |
| 进程 CPU tick 增量 | 此短窗口内为 0，不能解释为实际零开销 |

CPU 使用 `/proc/PID/stat` 的用户和内核 tick 除以墙钟时间，100% 表示一个 CPU 核心。短窗口和 tick 精度限制了结论。测试上游均在 loopback，不代表真实公网 DNS、TLS 或故障条件下的资源占用。

前端 gzip 数字来自构建工具，内置 HTTP 服务当前发送原始静态资源；反向代理可按需压缩。

## 未验证范围

- R4S BSP 上启动、ARM64 板级性能和长时间资源趋势。
- 真实 Mihomo 代理出口、Sub-Store 及订阅更新。
- 网络、DNS 解耦的线上迁移与代理停止后的实际联网。

ARM64 和 x86_64 的构建由 GitHub Actions 分别执行，结果以对应 workflow 为准。没有修改或部署线上路由器。
