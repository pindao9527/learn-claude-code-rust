# s11: Autonomous Agents (自主 Agent)

`s01 > s02 > s03 > s04 > s05 > s06 | s07 > s08 > s09 > s10 > [ s11 ] s12`

> *"队友自己看看板, 有活就认领"* -- 不需要领导逐个分配, 自组织。
>
> **Harness 层**: 自治 -- 模型自己找活干, 无需指派。

## 问题

s10 中队友只在被明确指派时才动。领导得给每个队友写 prompt，任务看板上 10 个未认领的任务得手动分配。这扩展不了。

真正的自治：队友自己扫描任务看板，认领没人做的任务，做完再找下一个。

一个细节：Context Compact (s06) 后 Agent 可能忘了自己是谁。**身份重注入**解决这个问题。

## 解决方案

```
Teammate lifecycle with idle cycle:

+-------+
| spawn |
+---+---+
    |
    v
+-------+   tool_use     +-------+
| WORK  | <------------- |  LLM  |
+---+---+                +-------+
    |
    | stop_reason != tool_calls (or idle tool called)
    v
+--------+
|  IDLE  |  poll every 5s for up to 60s
+---+----+
    |
    +---> check inbox --> message? ----------> WORK
    |
    +---> scan .tasks/ --> unclaimed? -------> claim -> WORK
    |
    +---> 60s timeout -----------------------> SHUTDOWN

Identity re-injection after compression:
  if messages.len() <= 3 {
      messages.insert(0, identity_block)
      messages.insert(1, assistant_ack)
  }
```

## 工作原理

1. 队友循环分两个阶段: WORK 和 IDLE。LLM 停止调用工具（或调用了 `idle`）时，进入 IDLE。

```rust
async fn _teammate_loop(name: String, role: String, ...) {
    'outer: loop {
        // -- WORK 阶段 --
        let mut idle_requested = false;
        for _ in 0..50 {
            // ... 调用 LLM, 执行工具 ...
            if idle_requested { break; }
            if finish_reason != "tool_calls" { break; }
        }

        // -- IDLE 阶段 --
        manager.set_status(&name, "idle");
        let mut resume = false;
        for _ in 0..12 {  // 12 * 5s = 60s
            sleep(Duration::from_secs(5)).await;
            // 检查信箱 / 扫描任务板 ...
            if resume { break; }
        }
        if !resume {
            manager.set_status(&name, "shutdown");
            return;
        }
        manager.set_status(&name, "working");
        // 回到 'outer loop 顶部继续 WORK
    }
}
```

2. IDLE 阶段轮询收件箱和任务看板。

```rust
// 检查信箱
let inbox = bus.read_inbox(&name);
if !inbox.is_empty() {
    for msg in &inbox {
        messages.push(Message::User { content: serde_json::to_string(msg).unwrap() });
    }
    resume = true;
    break;
}

// 扫描任务板
let unclaimed = scan_unclaimed_tasks(&tasks_dir);
if let Some(task) = unclaimed.first() {
    let task_id = task["id"].as_u64().unwrap_or(0);
    let result = claim_task(&tasks_dir, task_id, &name, &claim_lock);
    if !result.starts_with("Error:") {
        // 身份重注入（消息列表过短时）
        if messages.len() <= 3 {
            messages.insert(0, make_identity_block(&name, &role, &team_name));
            messages.insert(1, Message::Assistant { content: Some(format!("I am {}. Continuing.", name)), tool_calls: None });
        }
        messages.push(Message::User { content: format!("<auto-claimed>Task #{}: {}</auto-claimed>", task_id, task["subject"].as_str().unwrap_or("")) });
        resume = true;
        break;
    }
}
```

3. 任务看板扫描：找 pending 状态、无 owner、未被阻塞的任务。

```rust
fn scan_unclaimed_tasks(tasks_dir: &Path) -> Vec<Value> {
    let mut unclaimed = vec![];
    if let Ok(entries) = std::fs::read_dir(tasks_dir) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("task_"))
            .collect();
        paths.sort_by_key(|e| e.file_name());
        for entry in paths {
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                if let Ok(task) = serde_json::from_str::<Value>(&text) {
                    let is_pending = task["status"].as_str() == Some("pending");
                    let no_owner   = task["owner"].is_null();
                    let not_blocked = task["blockedBy"].as_array().map_or(true, |a| a.is_empty());
                    if is_pending && no_owner && not_blocked {
                        unclaimed.push(task);
                    }
                }
            }
        }
    }
    unclaimed
}
```

4. 身份重注入：上下文过短（说明发生了压缩）时，在开头插入身份块。

```rust
fn make_identity_block(name: &str, role: &str, team_name: &str) -> Message {
    Message::User {
        content: format!(
            "<identity>You are '{}', role: {}, team: {}. Continue your work.</identity>",
            name, role, team_name
        ),
    }
}
```

## Rust vs Python 关键差异

| 概念 | Python | Rust |
|------|--------|------|
| IDLE 等待 | `time.sleep(5)` | `tokio::time::sleep(Duration::from_secs(5)).await` |
| 认领互斥 | `threading.Lock()` | `Arc<Mutex<()>>` (锁不携带数据) |
| 外层循环跳转 | `while True: continue` | `'outer: loop { continue 'outer; }` |
| 目录遍历 | `Path.glob("task_*.json")` | `std::fs::read_dir` + `filter` + `sort_by_key` |
| JSON null 判断 | `not task.get("owner")` | `task["owner"].is_null()` |

## 相对 s10 的变更

| 组件           | 之前 (s10)       | 之后 (s11)                       |
|----------------|------------------|----------------------------------|
| Tools          | 12               | 14 (+idle, +claim_task)          |
| 自治性         | 领导指派         | 自组织                           |
| 空闲阶段       | 无               | 轮询收件箱 + 任务看板            |
| 任务认领       | 仅手动           | 自动认领未分配任务               |
| 身份           | 系统提示         | + 压缩后重注入                   |
| 超时           | 无               | 60 秒空闲 -> 自动关机            |

## 试一试

```sh
cd learn-claude-code-rust
cargo run --bin s11
```

试试这些 prompt：

1. `Create 3 tasks on the board, then spawn alice and bob. Watch them auto-claim.`
2. `Spawn a coder teammate and let it find work from the task board itself`
3. `Create tasks with dependencies. Watch teammates respect the blocked order.`
4. 输入 `/tasks` 查看带 owner 的任务看板
5. 输入 `/team` 监控谁在工作、谁在空闲
