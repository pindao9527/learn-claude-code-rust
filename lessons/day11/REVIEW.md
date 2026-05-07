# Day 11: Review & Key Concepts

## 1. 两阶段循环：WORK → IDLE → WORK

s11 最核心的结构改变是把单一的 `for _ in 0..30` 变成了双阶段无限循环：

```rust
loop {
    // == WORK 阶段 ==
    let mut idle_requested = false;
    for _ in 0..50 {
        // 调用 LLM，执行工具
        if idle_requested || should_exit { break; }
        if finish_reason != "tool_calls" { break; }
    }

    // == IDLE 阶段 ==
    manager.set_status(&name, "idle");
    let mut resume = false;
    for _ in 0..(IDLE_TIMEOUT / POLL_INTERVAL) {  // 60/5 = 12 次
        sleep(Duration::from_secs(POLL_INTERVAL)).await;
        // 检查信箱 / 扫描任务板
    }
    if !resume {
        manager.set_status(&name, "shutdown");
        return;
    }
    manager.set_status(&name, "working");
    // 回到 loop 顶部继续 WORK
}
```

**关键点**：Teammate 的退出不是从 WORK 阶段发生的，而是从 IDLE 阶段超时触发的。这让退出逻辑更可预测。

## 2. `tokio::time::sleep` vs `std::thread::sleep`

```rust
// ❌ 阻塞整个线程（其他 tokio 任务无法运行）
std::thread::sleep(Duration::from_secs(5));

// ✅ 只挂起当前 async task，其他任务继续运行
tokio::time::sleep(Duration::from_secs(POLL_INTERVAL)).await;
```

IDLE 阶段用 `tokio::time::sleep` 是必须的——如果用 `std::thread::sleep`，会把整个 tokio 线程池里的这个工作线程挂起，导致其他 Teammate 也无法推进。

## 3. `Arc<Mutex<()>>`：不携带数据的纯互斥锁

`claim_task` 需要保证"读-判断-写"三步原子执行，防止两个 Teammate 同时认领同一个任务：

```rust
fn claim_task(
    tasks_dir: &PathBuf,
    task_id: u64,
    owner: &str,
    claim_lock: &Arc<Mutex<()>>,  // () 表示锁本身不携带任何数据
) -> String {
    let _guard = claim_lock.lock().unwrap();
    // _guard 的生命周期覆盖整个函数体
    // 函数返回时 _guard drop，锁自动释放
    // ...
}
```

`Mutex<()>` 是 Rust 中"只需要互斥，不需要共享数据"的惯用法。

## 4. Identity Re-injection：对抗上下文遗忘

当消息历史很短（说明发生了上下文压缩），Teammate 可能忘记自己的身份。自动认领任务时检测并补注入：

```rust
if messages.len() <= 3 {
    messages.insert(0, make_identity_block(&name, &role, &team_name));
    messages.insert(1, Message::Assistant {
        content: Some(format!("I am {}. Continuing.", name)),
        tool_calls: None,
    });
}
```

**为什么要插入 Assistant 消息？** Claude 的 API 要求 User 和 Assistant 消息必须交替出现。如果连续两条 User 消息，API 会报错。

## 5. 运行时观察

今天实际运行时看到了一个真实问题：LLM 创建的任务文件格式（`.tasks/1.md`）与 `scan_unclaimed_tasks` 期望的格式（`task_N.json` + JSON 字段）不匹配，导致 Teammate 进入 IDLE 后无法找到可认领的任务。

`/team` 命令显示的结果：
```
alice (worker) status: shutdown   ← alice 超时后自动 shutdown
bob (worker) status: shutdown     ← 同上
coder (coder) status: idle        ← 还在 IDLE 等待中
worker1/2/3 status: idle
```

这说明**状态机是正常工作的**——Teammates 确实按照 WORK → IDLE → shutdown 的路径运行了。

## 6. `idle_requested` 标志位模式

`idle` 工具不是直接退出，而是设置标志位，让当前这一轮工具执行正常完成后再退出 WORK 阶段：

```rust
"idle" => {
    idle_requested = true;  // 设置标志
    "Entering idle phase.".to_string()  // 仍然返回工具结果给 LLM
},
// ...工具循环末尾：
if idle_requested { break; }  // 执行完所有工具后才退出
```

这保证了 LLM 能收到工具结果，对话历史是完整的。

## 💡 课后挑战

`/tasks` 命令目前只显示状态，试着扩展它：当 `status == "in_progress"` 时，同时显示认领时间（需要在 `claim_task` 里写入 `claimed_at` 字段）。

思考：如果两个 Teammate 同时读取到同一个 unclaimed 任务，`claim_lock` 能保证安全——但如果任务文件损坏（半写入），`claim_task` 会怎样？如何改进？
