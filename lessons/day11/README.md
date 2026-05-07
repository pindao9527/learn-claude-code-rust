# Day 11：自主 Agent (Autonomous Agents)

> *"Agent 不等指令——它自己找工作做。"* —— IDLE 轮询 + 任务板自动认领。

## 1. 目标

在 Day 10 中，我们实现了 Lead ↔ Teammate 的双向握手协议（Shutdown / Plan Approval）。  
今天我们给 Teammate 加上**自主性**：完成手头任务后不再等待，而是主动轮询任务板。

| 能力 | Day 10 | Day 11 |
|------|--------|--------|
| Teammate 启动 | 需要 Lead 显式传入任务 | 相同 |
| 完成任务后 | 直接退出 | 进入 IDLE，主动找下一个任务 |
| 任务来源 | 只能由 Lead 分配 | 也可从 `.tasks/` 任务板自动认领 |
| 上下文过长时 | 无处理 | 身份重注入（Identity Re-injection） |

---

## 2. Teammate 生命周期状态机

```
spawn
  ↓
┌────────────────────────────────────────────────────────┐
│                    WORK 阶段                            │
│  每轮：读信箱 → 调 LLM → 执行工具 → 循环              │
│  退出条件：stop_reason != tool_calls                   │
│            或 LLM 主动调用 idle 工具                   │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│                    IDLE 阶段                            │
│  每 5s 轮询一次，最多 60s（= 12 次）                   │
│  ├── 检查信箱 → 有消息 → break → 回到 WORK            │
│  ├── 扫描 .tasks/ → 有未认领任务 → 认领 → 回到 WORK   │
│  └── 12 次轮询全部空转 → shutdown                     │
└────────────────────────────────────────────────────────┘
```

---

## 3. 三个新概念

### 3.1 Identity Re-injection（身份重注入）

当消息列表很短（压缩后），Teammate 可能"忘记"自己是谁。  
解决方案：检测到消息 ≤ 3 条时，在最前面插入身份块：

```rust
// Python:
// messages = [identity_block, assistant_ack, ...remaining...]

// Rust 要实现：
fn make_identity_block(name: &str, role: &str, team_name: &str) -> Message {
    Message::User {
        content: format!(
            "<identity>You are '{}', role: {}, team: {}. Continue your work.</identity>",
            name, role, team_name
        ),
    }
}
```

### 3.2 `idle` 工具

Teammate 主动声明"我暂时没活干了"，触发进入 IDLE 阶段。

```
LLM 调用 idle 工具
  → 执行工具时设置标志位 idle_requested = true
  → 正常返回工具结果
  → 本轮 WORK 循环结束后检测标志位 → 进入 IDLE
```

### 3.3 任务板（Task Board）

`.tasks/task_N.json` 格式：

```json
{
  "id": 1,
  "subject": "写一个 Fibonacci 函数",
  "description": "用 Rust 实现并写测试",
  "status": "pending",
  "owner": null,
  "blockedBy": []
}
```

---

## 4. 今日编码任务（手动实现顺序）

### 第一步：复用 s10 全部基础代码

原样复制，不需要改动：
- `Message`、`InboxMessage`、`RequestStatus`、`ShutdownTracker`、`PlanTracker`
- `MessageBus`（含 `send_with_extra`）
- `TeammateManager`
- `run_bash`、`safe_path`、`run_read`、`run_write`、`run_edit`
- `handle_shutdown_request`、`handle_plan_review`

新增一行导入：

```rust
use tokio::time::{sleep, Duration};
```

### 第二步：实现 `scan_unclaimed_tasks`

```rust
fn scan_unclaimed_tasks(tasks_dir: &Path) -> Vec<Value> {
    // 1. 确保目录存在
    // 2. 读取所有 task_*.json 文件（用 read_dir + filter_map）
    // 3. 筛选：status == "pending" && owner 为 null/缺失 && blockedBy 为空
    // 4. 按文件名排序后返回
}
```

提示：`task["owner"].is_null()` 可判断 null；`task["blockedBy"].as_array().map_or(true, |a| a.is_empty())` 判断空数组。

### 第三步：实现 `claim_task`

```rust
fn claim_task(
    tasks_dir: &Path,
    task_id: u64,
    owner: &str,
    claim_lock: &Arc<Mutex<()>>,
) -> String {
    let _guard = claim_lock.lock().unwrap(); // 持锁直到函数结束
    // 1. 找到对应文件 task_{task_id}.json
    // 2. 检查 status / owner / blockedBy（同上）
    // 3. 修改 owner = owner, status = "in_progress"
    // 4. 写回文件
}
```

### 第四步：实现 `make_identity_block`

见 3.1 节，10 行以内。

### 第五步：改造 `_teammate_loop` 加入 IDLE 阶段

在现有 WORK 循环（`for _ in 0..50`）后，追加：

```rust
// -- IDLE 阶段 --
manager.set_status(&name, "idle");
let mut resume = false;
for _ in 0..12 {
    sleep(Duration::from_secs(5)).await;
    // 检查信箱 ...
    // 扫描任务板 ...
}
if !resume {
    manager.set_status(&name, "shutdown");
    return;
}
manager.set_status(&name, "working");
// 循环回到 WORK 阶段（把整个 loop 包在 'outer: loop { } 里）
```

### 第六步：工具列表新增 `idle` 和 `claim_task`

Teammate 工具新增：

```rust
{ "name": "idle",       "description": "Signal no more work. Enter idle polling.", ... }
{ "name": "claim_task", "description": "Claim a task from .tasks/ by ID.", ... }
```

Lead 工具也新增 `claim_task`（Lead 也可以认领任务）。

### 第七步：主循环新增 `/tasks` 斜线命令

```rust
"/tasks" => {
    // 遍历 .tasks/task_*.json，打印：
    // [ ] #1: subject (无主)
    // [>] #2: subject @owner
    // [x] #3: subject (completed)
}
```

---

## 5. Rust 知识点清单

| 概念 | 用途 | 本节示例 |
|------|------|---------|
| `tokio::time::sleep` | 异步非阻塞等待 | IDLE 轮询间隔 5s |
| `Arc<Mutex<()>>` | 互斥锁（不携带数据） | `claim_lock` 防止双重认领 |
| `loop { }` 外层循环 | WORK ↔ IDLE 循环切换 | `'outer: loop { ... break 'outer }` |
| `std::fs::read_dir` | 遍历目录文件 | 扫描 `.tasks/task_*.json` |
| `Value::is_null()` | 判断 JSON null | 检查 `owner` 字段 |

---

## 6. 关键理解：为什么需要 `claim_lock`？

```
问题：两个 Teammate 同时看到 task_1.json 的 owner 为空，
      都决定认领，结果两人同时写文件 → owner 被最后写的那个覆盖。

竞争条件（Race Condition）：
  Thread A: 读 → owner=null ✓ → （切换！）
  Thread B: 读 → owner=null ✓ → 写 owner="bob"
  Thread A:                    → 写 owner="alice"  ← 覆盖了 bob！

解决：claim_lock 保证"读-判断-写"原子执行：
  Thread A: 拿锁 → 读 → 判断 → 写 → 释放锁
  Thread B: 等锁 → 拿锁 → 读（已有 owner！）→ 返回 Error → 释放锁
```

这就是 **Check-Then-Act** 模式必须加锁的原因——也是 Rust 强迫你用 `Mutex` 的根本动机。
