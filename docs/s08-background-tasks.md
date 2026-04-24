# s08: Background Tasks (后台任务)

`s01 > s02 > s03 > s04 > s05 > s06 | s07 > [ s08 ] s09 > s10 > s11 > s12`

> *"慢操作丢后台, agent 继续想下一步"* -- 后台线程跑命令, 完成后注入通知。
>
> **Harness 层**: 后台异步执行 -- 模型继续思考, harness 负责并行和等待。

## 问题

有些命令要跑好几分钟: `npm install`、`cargo build`、`docker build`。阻塞式循环下模型只能干等。用户说 "装依赖, 顺便建个配置文件", Agent 却只能一个一个来，大大降低了效率。

## 解决方案

```text
Main Task                  Tokio Spawn (Background Tasks)
+-----------------+        +-----------------+
| agent loop      |        | async command   |
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

## 工作原理 (Rust 实现)

在 Rust 中，我们要应对所有权和跨任务(Task)状态共享的挑战。由于我们在基于 `tokio` 的异步环境中运行，我们需要使用 **`tokio::spawn`** 来启动后台任务，并使用 **`Arc<Mutex<...>>`** 来在多个 Task 之间安全地共享状态。

1. **共享的数据结构**: `BackgroundManager` 内部需要包裹一个可以在线程间共享的内部状态，包括正在运行的任务集合和一个通知队列。

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// 使用 Arc 包裹，允许内部变异并能在不同的 tokio task 间被 clone 传递
#[derive(Clone)]
pub struct BackgroundManager {
    // 任务状态映射
    pub tasks: Arc<Mutex<HashMap<String, BgTask>>>,
    // 执行完毕的通知队列
    pub notifications: Arc<Mutex<Vec<Notification>>>,
    // 简单的自增器替代 uuid
    next_id: Arc<Mutex<u32>>, 
}
```

2. **触发后台运行 (`run` 方法)**: `tokio::spawn` 接收一个 `Future` 并将其放在后台的线程池里调度，这样方法立即返回不会阻塞 Agent！

```rust
impl BackgroundManager {
    pub fn run(&self, command: String) -> String {
        // 生成唯一标识
        let mut id_lock = self.next_id.lock().unwrap();
        let task_id = format!("bg_{}", *id_lock);
        *id_lock += 1;
        drop(id_lock);

        // 记录状态
        self.tasks.lock().unwrap().insert(
            task_id.clone(), 
            BgTask { status: "running".to_string(), command: command.clone(), result: None }
        );

        // 克隆 Manager，它的内部是用 Arc 包裹的，所以代价极小，但赋予了所有权！
        let manager_clone = self.clone();
        
        // 发射后台任务！
        tokio::spawn(async move {
            // 在这里执行耗时的 Tokio 异步 Shell 命令
            // 执行完毕后获取结果，并通过 manager_clone.notifications.lock() 塞入通知
            manager_clone.execute_and_notify(task_id, command).await;
        });

        format!("Background task {} started", task_id_str)
    }
}
```

3. **每次 LLM 调用前排空通知队列**:

```rust
// 在 agent_loop 中
loop {
    // 掏空后台的积攒的完成通知
    let notifs = bg.drain_notifications();
    if !notifs.is_empty() {
        let mut notif_text = String::new();
        for n in notifs {
            notif_text.push_str(&format!("[bg:{}] {}\n", n.task_id, n.result));
        }
        messages.push(Message::User {
            content: format!("<background-results>\n{}\n</background-results>", notif_text)
        });
    }
    // 继续发起大模型调用...
}
```

循环保持单线程状态机。只有耗时的子进程被并行化在绿色线程里。

## 相对 s07 的变更

| 组件           | 之前 (s07)       | 之后 (s08)                         |
|----------------|------------------|------------------------------------|
| Tools          | 8                | `s07的所有功能` + `background_run` + `check_background` |
| 执行方式       | 仅阻塞 (`wait_timeout`) | 阻塞 + 异步后台(`tokio::spawn`)    |
| 通知机制       | 无               | 每轮排空的并发安全队列 (`drain`)   |
| 并发           | 无               | `tokio` 异步运行时调度             |

## 实践指南

通过这节课，你可以真正领略 Rust 中 **共享状态并发 (Shared-State Concurrency)** 的魅力，以及如何在编译器的严格把关下（要求类型满足 `Send` 和 `Sync`）写出绝不发生数据竞争的高效后台任务调度器！
