# Rust & Claude Code Harness 12天实战总纲

本总纲是你学习 Rust 深度特性的航标。每一天的课题都对应一个 Claude Code 的核心机制，你需要通过实现该机制来掌握对应的 Rust 底层原理。

---

## 第一阶段：基础、异步与类型（Day 1 - 3）

### Day 01: Agent Loop —— 异步基石
- **Harness 目标**：实现最简 `while` 循环，根据 LLM 的 `stop_reason` 决定是否继续。
- **Rust 深度解析点**：
    - **Async 状态机**：理解 `poll` 模型与 `Future` 的惰性执行。
    - **Tokio Runtime**：理解任务调度、工作线程池与协作式多任务。
    - **Error Handling**：`Result` 模式 vs 异常机制。

### Day 02: Tool Use —— 行为抽象
- **Harness 目标**：定义 `Tool` 接口，并使用 `HashMap` 实现工具分发。
- **Rust 深度解析点**：
    - **Trait 与 vtable**：理解静态分发 (`impl`) 与动态分发 (`dyn`) 的内存权衡。
    - **胖指针 (Fat Pointer)**：理解 `&dyn Tool` 的组成（对象地址 + 虚表地址）。
    - **Smart Pointers**：为什么需要 `Box<T>` 进行堆分配以实现异构集合。

### Day 03: TodoWrite —— 结构化数据
- **Harness 目标**：实现一个可持久化的任务清单结构体及文件读写工具。
- **Rust 深度解析点**：
    - **Ownership & Borrowing**：理解 `&self` 与 `&mut self` 的排他性。
    - **Serde 序列化**：理解零拷贝反序列化与 Rust 枚举类型的强一致性校验。

---

## 第二阶段：递归、路径与内存（Day 4 - 6）

### Day 04: Subagent —— 递归异步
- **Harness 目标**：在一个 Agent 循环中启动另一个独立的 Agent 实例处理子任务。
- **Rust 深度解析点**：
    - **BoxFuture**：如何解决异步函数递归调用时内存大小不确定的问题。
    - **Send + Sync**：理解跨线程传输数据的安全契约（为什么 `tokio::spawn` 需要 `Send`）。

### Day 05: Skill Loading —— 文本处理
- **Harness 目标**：按需从 `.md` 文件解析系统指令（Skills）并注入上下文。
- **Rust 深度解析点**：
    - **String vs &str**：理解分配所有权字符串与字符串切片的生命周期差异。
    - **PathBuf 安全性**：跨平台路径操作与文件系统权限处理的健壮性。

### Day 06: Context Compact —— 内存裁剪
- **Harness 目标**：当 Token 接近上限时，实施动态裁剪与摘要生成算法。
- **Rust 深度解析点**：
    - **Vec 内存布局**：理解 `Len` vs `Capacity`，以及 `drain/splice` 操作的性能代价。
    - **模式匹配枚举**：使用带数据的 `Enum` 实现复杂的压缩状态管理。

---

## 第三阶段：持久化、并发与图（Day 7 - 9）

### Day 07: Task System —— 关系建模
- **Harness 目标**：构建一个带依赖关系（Graph）的任务系统。
- **Rust 深度解析点**：
    - **关系建模**：为什么在 Rust 中使用 `ID Map` 比直接用 `Pointer` 表示图更好。
    - **DFS/BFS**：用 Rust 处理递归逻辑时的安全实践（避免栈溢出）。

### Day 08: Background Tasks —— 并发原语
- **Harness 目标**：在不阻塞对话的情况下启动长耗时工具，完成后通知 Agent。
- **Rust 深度解析点**：
    - **MPSC Channels**：多生产者单消费者的消息传递模型。
    - **Arc<Mutex<T>>**：理解原子引用计数与互斥锁的“共享+修改”模式。

### Day 09: Agent Teams —— 进程间通信
- **Harness 目标**：实现简单的“邮箱协议”，让多个 Agent 通过文件或 Socket 通信。
- **Rust 深度解析点**：
    - **JSONL 流式处理**：高效处理大规模日志和消息流。
    - **Signal Handling**：优雅退出与资源清理。

---

## 第四阶段：复杂调度与隔离（Day 10 - 12）

### Day 10: Team Protocols —— 协作协议
- **Harness 目标**：定义 Agent 之间的转单、确认和终止协议。
- **Rust 深度解析点**：
    - **Generics (泛型)**：编写高度通用的底层通信逻辑。
    - **Trait Bound**：利用 `Where` 子句进行复杂的类型约束。

### Day 11: Autonomous —— 事件驱动
- **Harness 目标**：Agent 进入“无人值守”模式，自动根据任务看板领活干。
- **Rust 深度解析点**：
    - **Tokio Select!**：学习如何同时监听多个异步流（用户输入、定时器、后台通知）。
    - **Polling**：理解事件冒泡与唤醒机制。

### Day 12: Isolation —— 系统安全
- **Harness 目标**：通过 Git Worktree 或环境变量实现任务执行环境的完全隔离。
- **Rust 深度解析点**：
    - **Process Control**：使用 `Command` 模块安全地执行外部 CLI 工具。
    - **Environment Sandboxing**：确保 Agent 的破坏性操作限制在最小权限范围内。

---
> **“掌握 Rust 只有一种方法：写下第一行代码，迎接第一个编译报错。”**
