# Rust & Claude Code Harness 12天实战总纲

本总纲是你系统学习 Rust 并复刻 Claude Code Agent 的航标。每一天的课题都将 Rust 的核心语言特性与 Agent 的核心机制深度结合。

---

## 总体进度表

| 天 | Rust 核心主题 | 复刻目标 | 课程文件 |
|----|-------------|---------|---------|
| Day 01 | 变量、类型、函数、控制流 | 跑通 `s01` 基础骨架 | [day01](./day01/) |
| Day 02 | 所有权 & 借用（Ownership & Borrowing） | 理解环境上下文 `Vec<Message>` 的传递 | [day02](./day02/) |
| Day 03 | 结构体 & 方法（struct / impl） | 把散函数重构为 `Agent` 结构体 | [day03](./day03/) |
| Day 04 | 枚举 & 模式匹配（enum / match） | 用 enum 替代 `stop_reason` 字符串比较 | [day04](./day04/) |
| Day 05 | 错误处理（Result / ? / thiserror） | 给所有错误路径加上有意义的类型 | [day05](./day05/) |
| Day 06 | Trait & 泛型 | 抽象 `LlmClient` trait，支持多后端 | [day06](./day06/) |
| Day 07 | 异步编程（async/await / tokio） | 深入理解 `s01` 里 async 运行时的任务调度 | [day07](./day07/) |
| Day 08 | 闭包 & 迭代器 | 用 `.map()/.filter()` 重写工具解析与历史裁剪 | [day08](./day08/) |
| Day 09 | 模块系统 & Cargo workspace | 拆分项目为 `core`, `tools`, `cli` 多个 Crate | [day09](./day09/) |
| Day 10 | 生命周期（Lifetime） | 优化性能，在工具输出引用中减少不必要的 Clone | [day10](./day10/) |
| Day 11 | 测试 & 调试 | 给 `agent_loop` 编写单元测试与集成测试 | [day11](./day11/) |
| Day 12 | 性能 & 并发 | 用 `tokio::spawn` 实现并发工具调用与后台监控 | [day12](./day12/) |

---

## 阶段详情

### 第一阶段：起步与核心所有权 (Day 01 - Day 03)

*   **Day 01: 基础骨架** - 实现最简 `while` 循环，调用 LLM API。
*   **Day 02: 内存安全金钥匙** - 深入所有权。Agent 历史记录 `Vec` 如何在函数间流转而不引发竞态或内存泄漏。
*   **Day 03: 封装的力量** - 使用 `struct` 维护 Agent 状态，用 `impl` 定义其行为。

### 第二阶段：强类型与健壮性 (Day 04 - Day 06)

*   **Day 04: 优雅的分发** - 使用 `Enum` 定义 `StopReason` (ToolUse, EndTurn) 和 `Tool` 类型，通过 `match` 实现安全的分发。
*   **Day 05: 消除恐慌** - 抛弃 `unwrap()`。学习使用 `thiserror` 定义 Agent 业务错误，利用 `?` 操作符传播错误。
*   **Day 06: 高级抽象** - 编写 `LlmClient` Trait。你的 Agent 将不再绑定到特定供应商，实现插件化。

### 第三阶段：异步、迭代与架构 (Day 07 - Day 09)

*   **Day 07: 异步之心** - `tokio` 深度解析。理解为什么 Agent 循环必须是异步的，以及什么是 `Future`。
*   **Day 08: 函数式编程** - 使用迭代器链式处理上下文，优雅地过滤无效信息或提取工具参数。
*   **Day 09: 工程化工程** - 使用 Cargo Workspace 管理多包项目，让代码从脚本进化为专业工程。

### 第四阶段：深度优化与生产力 (Day 10 - Day 12)

*   **Day 10: 触碰底层** - 当你需要跨作用域引用数据时，学习标明生命周期。优化 Token 计算的内存占用。
*   **Day 11: 信心保证** - 学习 Rust 的测试哲学。模拟 (Mock) LLM 响应，确保 Agent 在各种边界条件下都能正确运行。
*   **Day 12: 极致并发** - 真正的 Claude Code 能够并发执行工具。学习 `spawn` 任务，并使用 `mpsc` 通道进行状态同步。

---
> **“掌握 Rust 只有一种方法：写下第一行代码，迎接第一个编译报错。”**
