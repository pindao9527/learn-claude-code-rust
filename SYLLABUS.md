# Rust & Claude Code Harness 19天实战总纲

本总纲是你系统学习 Rust 并复刻 Claude Code Agent 的航标。
每一天对应原版 Python 项目的一个章节（s01~s19），Rust 核心主题服务于当天的复刻目标。

---

## 总体进度表

| 天   | 复刻目标        | Rust 核心主题                          | 课程文件         |
|------|----------------|---------------------------------------|-----------------|
| Day 01 | `s01` 基础 Agent 循环 | 变量、类型、函数、控制流、async 入门 | [day01](./day01/) |
| Day 02 | `s02` 工具调用（Tool Use） | 所有权 & 借用、枚举、Serde | [day02](./day02/) |
| Day 03 | `s03` TodoWrite 会话规划 | 结构体 & 方法（struct / impl） | [day03](./day03/) |
| Day 04 | `s04` 子 Agent（Subagent） | 枚举 & 模式匹配（enum / match） | [day04](./day04/) |
| Day 05 | `s05` 技能加载（Skill Loading） | 错误处理（Result / ? / thiserror） | [day05](./day05/) |
| Day 06 | `s06` 上下文压缩（Context Compact） | Trait & 泛型 | [day06](./day06/) |
| Day 07 | `s07` 权限系统（Permission System） | 异步编程深入（async/await / tokio） | [day07](./day07/) |
| Day 08 | `s08` Hook 系统 | 闭包 & 迭代器 | [day08](./day08/) |
| Day 09 | `s09` 记忆系统（Memory System） | 模块系统 & Cargo workspace | [day09](./day09/) |
| Day 10 | `s10` 系统提示词组装（System Prompt） | 生命周期（Lifetime） | [day10](./day10/) |
| Day 11 | `s11` 错误恢复（Error Recovery） | 测试 & 调试 | [day11](./day11/) |
| Day 12 | `s12` 任务系统（Task System / DAG） | 文件 I/O & serde_json 进阶 | [day12](./day12/) |
| Day 13 | `s13` 后台任务（Background Tasks） | 线程 & `std::sync`（Mutex / Arc） | [day13](./day13/) |
| Day 14 | `s14` 定时调度（Cron Scheduler） | 时间处理（`chrono` / `tokio::time`） | [day14](./day14/) |
| Day 15 | `s15` Agent 团队（Agent Teams） | 进程管理 & 文件锁 | [day15](./day15/) |
| Day 16 | `s16` 团队协议（Team Protocols） | 状态机（State Machine with enum） | [day16](./day16/) |
| Day 17 | `s17` 自主 Agent（Autonomous Agents） | `tokio::spawn` & 任务调度 | [day17](./day17/) |
| Day 18 | `s18` Worktree 任务隔离 | 子进程 & `std::process::Command` | [day18](./day18/) |
| Day 19 | `s19` MCP 插件系统 | 协议设计 & stdio 通信 | [day19](./day19/) |

---

## 阶段详情

### 第一阶段：起步与核心所有权 (Day 01 - Day 03)

- **Day 01: 基础骨架** — 实现最简 `while` 循环，调用 LLM API，理解 async/await 基础。
- **Day 02: 内存安全金钥匙** — 深入所有权。Agent 历史记录 `Vec<Message>` 如何在函数间流转，工具调用的枚举建模。
- **Day 03: 封装的力量** — 用 `struct` 维护 `TodoManager` 状态，用 `impl` 定义其行为。第一次体验"数据 + 行为"分离。

### 第二阶段：强类型与健壮性 (Day 04 - Day 06)

- **Day 04: 优雅的分发** — 用 `enum` 建模子 Agent 的输入/输出，通过 `match` 实现安全分发。理解递归调用与所有权。
- **Day 05: 消除恐慌** — 抛弃 `unwrap()`。用 `thiserror` 定义技能加载的业务错误，`?` 操作符传播错误。
- **Day 06: 高级抽象** — 编写 `Compressor` Trait，用泛型让压缩策略可替换。

### 第三阶段：异步、迭代与架构 (Day 07 - Day 09)

- **Day 07: 异步之心** — 权限系统的四阶段 pipeline 天然适合 async 链式调用。深入理解 `Future` 与任务调度。
- **Day 08: 函数式编程** — Hook 系统的事件分发用迭代器链优雅实现。闭包作为回调的惯用法。
- **Day 09: 工程化工程** — 记忆系统需要文件扫描与 YAML 解析，借此拆分项目为多模块，理解 `mod` 与 `pub`。

### 第四阶段：持久化与并发基础 (Day 10 - Day 12)

- **Day 10: 提示词流水线** — 系统提示词的分段组装，理解字符串切片与生命周期的实际场景。
- **Day 11: 信心保证** — 给错误恢复的三条分支编写单元测试，Mock LLM 响应。
- **Day 12: 图结构数据** — 任务依赖图（DAG）的 JSON 持久化，`serde` 的高级用法（自定义序列化）。

### 第五阶段：并发、调度与多 Agent (Day 13 - Day 16)

- **Day 13: 线程安全** — 后台任务用 `Arc<Mutex<>>` 共享通知队列，理解 Rust 并发的核心保证。
- **Day 14: 时间驱动** — 定时调度器用 `tokio::time::interval` 实现，理解异步定时器与 cron 表达式解析。
- **Day 15: 进程协作** — Agent 团队通过 JSONL 文件通信，理解文件锁与 append-only 写入模式。
- **Day 16: 状态机** — 用 `enum` + `match` 实现请求/响应协议的状态机，这是 Rust 最优雅的模式之一。

### 第六阶段：自主性与扩展性 (Day 17 - Day 19)

- **Day 17: 真正的并发** — 自主 Agent 用 `tokio::spawn` 独立运行，`mpsc` channel 同步状态。
- **Day 18: 进程隔离** — Worktree 管理通过 `std::process::Command` 调用 git，理解子进程的 stdin/stdout 捕获。
- **Day 19: 协议设计** — MCP 插件通过 stdio 通信，实现一个最小化的 JSON-RPC 客户端，理解协议边界。

---

## 已完成进度

| 天    | 状态 |
|-------|------|
| Day 01 | ✅ 完成 |
| Day 02 | ✅ 完成 |
| Day 03 | ✅ 完成 |
| Day 04 | ✅ 完成 |
| Day 05 | ✅ 完成 |
| Day 06 | ✅ 完成 |
| Day 07 | ✅ 完成 |

---

> **"掌握 Rust 只有一种方法：写下第一行代码，迎接第一个编译报错。"**
