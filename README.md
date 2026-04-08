# learn-claude-code-rust

> 用 Rust 重写 [learn-claude-code](https://github.com/shareAI-lab/learn-claude-code) 的全部 Agent Harness 课程。

[Python 原版](https://github.com/shareAI-lab/learn-claude-code) | [中文文档](https://github.com/shareAI-lab/learn-claude-code/blob/main/README-zh.md)

---

## 一句话说清楚这个仓库在做什么

原版 `learn-claude-code` 用 Python 逐步实现了 12 个 Agent Harness 机制（从最小 agent loop 到 worktree 隔离的自治多 agent 团队）。

本仓库从头用 **Rust** 移植相同的 12 个课程，目标是：

1. **学 Rust**：在真实项目中掌握 async/await、所有权、trait、错误处理等核心概念；
2. **学 Harness 工程**：理解 Agent 不是框架而是模型，代码的职责只是给模型造能用的工具和环境；
3. **体会两种语言的差异**：同一个逻辑，Python 3 行，Rust 30 行 — 不是坏事，是对类型系统和并发安全的投资。

---

## 项目哲学：学习导向（Learning First）

本项目不是为了生成一个完美的 Rust 版 Claude Code，而是为了：

1. **重构中学习**：通过将 Python 代码移植到 Rust，深度理解 Rust 的所有权（Ownership）、异步（Async）和类型系统。
2. **原理拆解**：每一课都对应 Claude Code 的一个核心 Harness 机制，通过手动实现来彻底掌握其工作原理。

**注意**：AI 助手已被约束，在本项目中优先进行教导和思路引导，而不是直接自动补全所有代码。

---

## 目录结构

- **`docs/`**: Claude Code 原理文档、技术规范及原版功能说明。
- **`lessons/`**: Rust 语言学习目录，记录在重写过程中涉及的 Rust 核心概念和练习。
- **`agents/`**: Claude Code 的核心实现目录，按 s01-s12 逐步推进。

---

## 前置知识

你不需要是 Rust 专家。但你需要：

- 能读懂 Python（原版参考实现）
- 装好 Rust 工具链（见下方快速开始）
- 有一个支持 OpenAI 兼容接口的 API Key（Anthropic / OpenAI / DeepSeek 均可）

---

## 为什么用 Rust 重写

Python 让你专注逻辑；Rust 让你在编译期就把并发问题排除掉。

对于 Agent Harness，Rust 的优势体现在：

- **多 agent 并发**：`tokio` 的 async 任务比 Python `asyncio` 更轻量，线程安全由编译器保证；
- **长期运行稳定性**：没有 GIL，没有意外的内存泄漏；
- **部署简单**：单一静态二进制，无需 Python 虚拟环境。

代价是学习曲线更陡。这正是本仓库存在的理由。

---

## 许可证

MIT

---

**模型就是 Agent。代码是 Harness。造好 Harness，Agent 会完成剩下的。**
