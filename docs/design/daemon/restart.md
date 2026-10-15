# Daemon 配置触发的网关重启

## 概述

配置中属「重启生效类」的变更（判定标准见 [config 需求 §F7](../../requirements/config.md)）经确认后，由 Daemon 在无活跃会话的择机窗口自动执行无损重启——重建承载重启类配置的组件（Gateway、LLM Registry 等）并重新接线，会话层与对话状态不受影响，无需 Owner 手动操作。

## 架构

重启类配置变更的判定（判定标准与归属矩阵）依据 [config 需求 §F7](../../requirements/config.md)（生效机制与重启类判定），变更确认见 [config 需求 §F4](../../requirements/config.md)（配置重载）与 [config 热重载](../config/hot-reload.md)（重启类变更确认与触发）；需求侧行为见 [daemon 需求 §F6](../../requirements/daemon.md)（配置触发的网关重启）。确认后由 Daemon 择机执行无损重启：

- **待重启状态**：Daemon 进入待重启状态并记录待生效变更，系统正常运行——会话正常处理消息、新消息不受影响，重启执行前继续按旧配置运行；进入待重启状态及开始等待择机窗口时经 IM 通知 Owner（等待期不再重复通知）
- **变更合并**：待重启期间（重启执行前）新到达的重启类变更并入同一次待执行集合，不重复重启；重建执行期间到达的重启类变更并入下一次待执行集合
- **择机窗口**：Daemon 经 SessionManager 查询全部会话的四维活跃维度（与关闭流程同一套判定，见 [shutdown.md](shutdown.md)），全部为否即满足窗口；无活跃会话时立即执行。窗口判定为瞬时快照，判定通过即进入重建执行期——此后的入站消息按重建期暂存补投处理（见 [gateway 需求 §F6](../../requirements/gateway.md)）
- **重建集合**：会话层完全不动，重启生效类配置项到重建对象的映射如下（其余组件不重建）：
  - Gateway 与 Plugin Registry（服务参数、机器人↔Agent 绑定）：任何重启都重建 Gateway 与 Plugin Registry；Plugin Registry 全量重建后，各平台插件（含不在 `platforms/` 下、经显式注册加入的终端渠道 CLI Chat `terminal` 插件）随对应平台连接的重建而重新注册并初始化，见 [gateway](../gateway/README.md)
  - Renderers / Plugins 与 IM Adapters（新增/停用平台的 Renderer、Plugin 封装与 Adapter 创建及连接建立/关闭）：仅当本次重启由平台启用清单变更触发时才发生，见 [im_adapter](../im_adapter/README.md)
  - LLM Registry 与 LlmCaller 接线（模型与凭据定义，见 [README 启动路径](README.md)）——经可替换的间接层重新接线，各 session 持有的对话状态不变
  - 因持有上述引用而需随之重建的下游组件（Admin RPC Server——其管理接口连接在重建期间断开）：CLI Admin 在管理接口不可达时报错退出、不重连，terminal 渠道的连接随该管理接口 socket 重建而断开（见 [cli/admin.md](../cli/admin.md)）
- **重建执行**：重建期间的入站消息由 Gateway 的持久化入站队列承载并跨越重建窗口，新 Gateway 就绪后按原到达顺序补投（见 [gateway 需求 §F6](../../requirements/gateway.md)、[gateway 消息队列](../gateway/README.md)）；执行中的出站消息按 graceful 语义收尾（等待在途出站发送完成，不涉及 forceful 场景）
- **重建失败兜底**：新组件初始化或接线失败、或就绪后的基本可用性校验（如 LLM 能力可用性）不通过时，中止本次重建，保留旧组件与旧配置继续运行，并经 IM 通知 Owner（含失败原因，见 [daemon 需求 §F6](../../requirements/daemon.md)）、不得静默失败——避免「重建构建成功但系统不可用」被误报为重启成功；待生效变更保留在暂存区，待下次择机窗口重试；会话层始终不受影响
- **单平台失败**：单个平台插件初始化失败时按 [im_adapter](../im_adapter/README.md) 记日志并跳过该平台、不中止整次重建，其余平台正常生效；完成通知中标注未生效的平台
- **无窗口兜底**：存在活跃会话时持续等待、不强制打断任何会话。Owner 可随时终止系统——系统处于待重启状态或重建执行期间收到关闭信号即直接进入 forceful 模式（中止待执行/进行中的重建，见 [shutdown.md](shutdown.md)），下次启动时配置自然生效
- **完成通知**：重启完成后 config 模块将**本次待执行集合**对应的暂存新值切换为运行时配置生效（重建执行期间并入下一批的变更不在此次切换范围，见 [config/hot-reload](../config/hot-reload.md)），并经 IM 通知 Owner，附本次生效的配置变更概要
- **与关闭流程的互斥**：系统进入关闭流程后不再触发/执行网关重启，待执行重启随之取消；网关重启也不在关闭流程中执行（见 [shutdown.md](shutdown.md)）

## 数据流

配置模块确认重启类变更 → Daemon 进入待重启状态（系统正常运行，新消息不受影响）→ 择机窗口满足（经 SessionManager 查全部会话四维活跃维度均为否）→ 重建承载重启类配置的组件（Gateway、LLM Registry 等）并重新接线（会话层不动；入站消息由 Gateway 持久化入站队列承载）→ 补投暂存消息、出站按 graceful 语义收尾（等待在途出站发送完成）→ 经 IM 通知 Owner（附本次生效的配置变更概要）。

## 模块关系

### 上游

- [config 模块](../config/README.md)：变更确认后触发本机制（[hot-reload](../config/hot-reload.md)「重启类变更确认与触发」），并携带待生效变更信息；重启完成后将待重启暂存的新值切换为运行时配置生效

### 下游

- [SessionManager](../session/README.md)：提供全部会话的四维活跃维度查询（择机窗口判定）
- [Gateway](../gateway/README.md)：被重建对象；重建期间的入站消息由其持久化入站队列承载
- Admin RPC Server：随 Gateway 重建——管理接口连接在重建期间断开，CLI Chat / CLI Admin 按各自重连语义处理
- LLM Registry（Daemon 装配的组件）：随重建重新装配，承载模型与凭据定义并重新接线 LlmCaller（经可替换的间接层，会话层不动）
- [IM Adapters](../im_adapter/README.md)（重建侧）：平台启用清单变更时随重建创建/关闭对应平台 Adapter 与连接
- [IM Adapters](../im_adapter/README.md)：发送重启完成 / 失败通知（失败通知含失败原因），并在重建期间按 graceful 语义收尾在途出站消息

### 与同模块子功能的关系

- [shutdown.md](shutdown.md)：网关重启不经过关闭流程、会话层不动；「无窗口兜底」下 Owner 可改用强制关闭，改走 shutdown.md 的 forceful 流程（本次待执行重启随之取消）
