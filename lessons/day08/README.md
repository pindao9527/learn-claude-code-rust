# Day 08: 后台任务系统 (Background Tasks)

> *"慢操作丢后台, agent 继续想下一步"* -- 后台线程跑命令, 完成后注入通知。

## 1. 目标

Agent 经常需要执行一些耗时极长的命令，比如 `npm install`、`cargo build`。
在阻塞模式下，这些命令执行期间，大模型什么也干不了。今天我们的目标是赋予 Agent **异步后台执行**的能力。

## 2. 架构转变

从阻塞单线程到多任务并发。

```text
Main Task                  Tokio Spawn (Background Tasks)
+-----------------+        +-----------------+
| agent_loop      |        | async command   |
| ...             |        | ...             |
| [LLM call] <---+------- | enqueue(result) |
|  ^drain queue   |        +-----------------+
+-----------------+

Timeline:
Agent --[spawn A]--[spawn B]--[other work]----
             |          |
             v          v
          [A runs]   [B runs]      (parallel)
             |          |
             +-- results injected before next LLM call --+
```

## 3. Rust 中的实现挑战

在 Python 中，这非常简单，开个 `threading.Thread` 然后维护一个全局数组即可。
但在 Rust 中，我们面临着**共享状态并发 (Shared-State Concurrency)** 的考验：

1. **跨线程/Task 读写**：主 Agent 需要循环读队列，而后台任务需要随机向队列塞结果。
2. **所有权转移**：闭包如何带走外部的数据？

为了解决这些问题，我们引入了 Rust 并发编程的利器：**`Arc<Mutex<T>>`** 以及 Tokio 的 **`spawn_blocking`**。

## 4. 今日任务

1. 创建 `BackgroundManager` 封装安全状态。
2. 提供 `run` 方法利用 `tokio::task::spawn_blocking` 将耗时命令发配到专属线程。
3. 改造老大哥 `agent_loop`，每次循环开头主动排空 `drain_notifications` 并通知 LLM。
4. 注册 `background_run` 和 `check_background` 新工具。

> **架构注记**：我们讨论确认了，子 Agent (`run_subagent`) 寿命极短且没有排空读取机制，因此**不应该**具备触发后台任务的权限，这体现了严谨的系统权限分工。
