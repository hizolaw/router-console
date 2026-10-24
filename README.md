# Router Console

面向低资源 Linux 路由器的模块化 Web 控制台。

目标是统一展示网络、DNS、Mihomo、订阅和系统状态，并通过可选插件提供受限的管理操作。当前处于设计阶段，尚无可运行版本。

## 设计原则

- 基础网络服务独立运行，控制台停止不影响 DNS、路由或订阅定时更新。
- 插件分别控制启用和展示；隐藏卡片不等于停止监测，故障服务不能自动消失。
- Go 后端嵌入静态前端；Preact、TypeScript、Vite 只参与前端构建。
- 采集缓存共享，历史数据有界，探测有超时和退避。
- 默认普通用户运行，管理操作使用显式授权，不提供任意命令执行。
- 不存储部署专用地址、订阅、节点或凭据到 Git。

## 计划中的模块

| 模块 | 功能 |
| --- | --- |
| System | CPU、内存、温度、存储、运行时间 |
| Network | 接口、网关、流量和连通性 |
| DNS | 服务状态与实际解析探测 |
| Mihomo | 进程、API、代理出口与 Zashboard 入口 |
| Subscriptions | Sub-Store 入口、更新结果、手动触发更新 |
| udpxy | 可选服务状态与入口 |

## 仓库边界

本仓库维护控制台代码、插件接口、通用示例和构建定义。NixOS 路由器仓库负责服务组合、机器地址、硬件、网络拓扑和部署。控制台不承担网络故障恢复职责。

详细方案见 [架构设计](docs/architecture.md)。

## 参考项目

- [Glance](https://github.com/glanceapp/glance)：Go 单程序、组件配置和缓存。
- [Homepage](https://github.com/gethomepage/homepage)：服务卡片和后端 API 集成。
- [Zashboard](https://github.com/Zephyruso/zashboard)：复用 Mihomo 管理界面。
- [Sub-Store](https://github.com/sub-store-org/Sub-Store)：独立订阅管理。

以上是设计参考与集成对象，本仓库当前没有复制这些项目的代码或资源。
