# Rust & Claude Code Harness 12天实战总纲

本总纲是你系统学习 Rust 并复刻 Claude Code Agent 的航标。
参考项目为 [learn-claude-code](https://github.com/shareAI-lab/learn-claude-code)（Python 版，共 s01~s12）。

---

## 总体进度表

| 天     | 复刻目标                              | Rust 核心主题                              | 参考 Python 文件 |
|--------|--------------------------------------|--------------------------------------------|-----------------|
| Day 01 | `s01` 基础 Agent 循环                 | 变量、类型、函数、控制流、async 入门        | `s01_agent_loop.py` |
| Day 02 | `s02` 工具调用（Tool Use）            | 所有权 & 借用、枚举、Serde                  | `s02_tool_use.py` |
| Day 03 | `s03` TodoWrite 会话规划              | 结构体 & 方法（struct / impl）              | `s03_todo_write.py` |
| Day 04 | `s04` 子 Agent（Subagent）            | 枚举 & 模式匹配（enum / match）             | `s04_subagent.py` |
| Day 05 | `s05` 技能加载（Skill Loading）       | 错误处理（Result / ? / thiserror）          | `s05_skill_loading.py` |
| Day 06 | `s06` 上下文压缩（Context Compact）   | Trait & 泛型                               | `s06_context_compact.py` |
| Day 07 | `s07` 任务系统（Task System / DAG）   | 文件 I/O & serde_json & PathBuf             | `s07_task_system.py` |
| Day 08 | `s08` 后台任务（Background Tasks）    | 线程 & `std::sync`（Mutex / Arc）           | `s08_background_tasks.py` |
| Day 09 | `s09` Agent 团队（Agent Teams）       | 进程协作 & 文件锁（append-only JSONL）      | `s09_agent_teams.py` |
| Day 10 | `s10` 团队协议（Team Protocols）      | 状态机（State Machine with enum）           | `s10_team_protocols.py` |
| Day 11 | `s11` 自主 Agent（Autonomous Agents） | `tokio::spawn` & 任务调度 & mpsc channel    | `s11_autonomous_agents.py` |
| Day 12 | `s12` Worktree 任务隔离               | 子进程 & `std::process::Command`            | `s12_worktree_task_isolation.py` |

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

### 第三阶段：持久化与并发基础 (Day 07 - Day 09)

- **Day 07: 图结构数据** — 任务依赖图（DAG）的 JSON 持久化，`serde` 的高级用法，文件 I/O 与 PathBuf。对应 `s07_task_system.py`。
- **Day 08: 线程安全** — 后台任务用 `Arc<Mutex<>>` 共享通知队列，理解 Rust 并发的核心保证。对应 `s08_background_tasks.py`。
- **Day 09: 进程协作** — Agent 团队通过 JSONL 文件通信，理解文件锁与 append-only 写入模式。对应 `s09_agent_teams.py`。

### 第四阶段：多 Agent 协作 (Day 10 - Day 12)

- **Day 10: 状态机** — 用 `enum` + `match` 实现 shutdown / plan-approval 协议的状态机，这是 Rust 最优雅的模式之一。对应 `s10_team_protocols.py`。
- **Day 11: 真正的并发** — 自主 Agent 用 `tokio::spawn` 独立运行，`mpsc` channel 同步状态，idle 轮询任务板。对应 `s11_autonomous_agents.py`。
- **Day 12: 进程隔离** — Worktree 管理通过 `std::process::Command` 调用 git，理解子进程的 stdin/stdout 捕获，任务与 worktree 双向绑定。对应 `s12_worktree_task_isolation.py`。

---

## 已完成进度

| 天     | 状态 |
|--------|------|
| Day 01 | ✅ 完成 |
| Day 02 | ✅ 完成 |
| Day 03 | ✅ 完成 |
| Day 04 | ✅ 完成 |
| Day 05 | ✅ 完成 |
| Day 06 | ✅ 完成 |
| Day 07 | ✅ 完成 |
| Day 08 | ✅ 完成 |
| Day 09 | ✅ 完成 |
| Day 10 | ✅ 完成 |
| Day 11 | ⬜ 待开始 |
| Day 12 | ⬜ 待开始 |

---

> **"掌握 Rust 只有一种方法：写下第一行代码，迎接第一个编译报错。"**
