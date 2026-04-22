# s07: Task System (任务系统)

`s01 > s02 > s03 > s04 > s05 > s06 | [ s07 ] s08 > s09 > s10 > s11 > s12`

> *"大目标要拆成小任务, 排好序, 记在磁盘上"* -- 文件持久化的任务图, 为多 agent 协作打基础。
>
> **Harness 层**: 持久化任务 -- 比任何一次对话都长命的目标。

## 问题

s03 的 TodoManager 只是内存中的扁平清单: 没有顺序、没有依赖、状态只有做完没做完。真实目标是有结构的 -- 任务 B 依赖任务 A, 任务 C 和 D 可以并行, 任务 E 要等 C 和 D 都完成。

没有显式的关系, Agent 分不清什么能做、什么被卡住、什么能同时跑。而且清单只活在内存里, 上下文压缩 (s06) 一跑就没了。

## 解决方案

把扁平清单升级为持久化到磁盘的**任务图**。每个任务是一个 JSON 文件, 有状态、前置依赖 (`blockedBy`)。任务图随时回答三个问题:

- **什么可以做?** -- 状态为 `pending` 且 `blockedBy` 为空的任务。
- **什么被卡住?** -- 等待前置任务完成的任务。
- **什么做完了?** -- 状态为 `completed` 的任务, 完成时自动解锁后续任务。

```
.tasks/
  task_1.json  {"id":1, "status":"completed"}
  task_2.json  {"id":2, "blockedBy":[1], "status":"pending"}
  task_3.json  {"id":3, "blockedBy":[1], "status":"pending"}
  task_4.json  {"id":4, "blockedBy":[2,3], "status":"pending"}

任务图 (DAG):
                 +----------+
            +--> | task 2   | --+
            |    | pending  |   |
+----------+     +----------+    +--> +----------+
| task 1   |                          | task 4   |
| completed| --> +----------+    +--> | blocked  |
+----------+     | task 3   | --+     +----------+
                 | pending  |
                 +----------+

顺序:   task 1 必须先完成, 才能开始 2 和 3
并行:   task 2 和 3 可以同时执行
依赖:   task 4 要等 2 和 3 都完成
状态:   pending -> in_progress -> completed
```

这个任务图是 s07 之后所有机制的协调骨架: 后台执行 (s08)、多 agent 团队 (s09+)、worktree 隔离 (s12) 都读写这同一个结构。

## 工作原理 (Rust 实现)

### 1. 数据结构：Task + TaskManager

任务本身是一个可序列化的结构体，通过 `serde` 自动实现 JSON 读写：

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Task {
    pub id: u32,
    pub subject: String,
    pub description: String,
    pub status: String,           // "pending" | "in_progress" | "completed"
    pub blocked_by: Vec<u32>,     // 依赖的任务 ID 列表
    pub owner: String,
}

pub struct TaskManager {
    pub dir: PathBuf,
    next_id: u32,                 // 私有字段，外部不可见
}
```

**Rust 特性体现**：
- `#[derive(Serialize, Deserialize)]` — 自动生成 JSON 序列化代码
- `next_id` 私有 — 防止外部直接修改，保证 ID 分配的一致性
- `PathBuf` — 跨平台的路径类型，比字符串更安全

### 2. 初始化：扫描目录计算 next_id

```rust
impl TaskManager {
    pub fn new(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        let next_id = Self::max_id(&dir) + 1;
        Self { dir, next_id }
    }

    fn max_id(dir: &PathBuf) -> u32 {
        let mut max = 0u32;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let s = name.to_string_lossy();
                // 从 "task_3.json" 中提取数字 3
                if let Some(n) = s.strip_prefix("task_")
                    .and_then(|s| s.strip_suffix(".json"))
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    if n > max { max = n; }
                }
            }
        }
        max
    }
}
```

**Rust 特性体现**：
- `flatten()` — 过滤掉读取失败的条目
- `strip_prefix/suffix` — 安全的字符串处理，返回 `Option`
- `and_then` 链式调用 — 任何一步失败都会短路返回 `None`

### 3. 持久化：load 和 save

```rust
fn load(&self, task_id: u32) -> Result<Task, String> {
    let path = self.dir.join(format!("task_{}.json", task_id));
    let content = std::fs::read_to_string(&path)
        .map_err(|_| format!("Task {} not found", task_id))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Parse error: {}", e))
}

fn save(&self, task: &Task) -> Result<(), String> {
    let path = self.dir.join(format!("task_{}.json", task.id));
    let json = serde_json::to_string_pretty(task)
        .map_err(|e| format!("Serialize error: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Write error: {}", e))
}
```

**Rust 特性体现**：
- `Result<T, E>` — 显式的错误处理，不会 panic
- `map_err` — 错误类型转换，把 `io::Error` 转成 `String`
- `?` 操作符 — 错误传播，失败时提前返回

### 4. 创建任务：create

```rust
pub fn create(&mut self, subject: String, description: String) -> String {
    let task = Task {
        id: self.next_id,
        subject,
        description,
        status: "pending".to_string(),
        blocked_by: vec![],
        owner: String::new(),
    };
    self.save(&task).ok();
    self.next_id += 1;
    serde_json::to_string_pretty(&task).unwrap_or_default()
}
```

**Rust 特性体现**：
- `&mut self` — 可变借用，允许修改 `next_id`
- `.ok()` — 显式忽略 `save` 的错误（转成 `Option` 后丢弃）

### 5. 依赖解除：clear_dependency

完成任务时，遍历所有任务文件，从 `blocked_by` 中移除已完成的任务 ID：

```rust
fn clear_dependency(&self, completed_id: u32) {
    if let Ok(entries) = std::fs::read_dir(&self.dir) {
        let ids: Vec<u32> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.strip_prefix("task_")
                    .and_then(|s| s.strip_suffix(".json"))
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .collect();
        for id in ids {
            if let Ok(mut task) = self.load(id) {
                if task.blocked_by.contains(&completed_id) {
                    task.blocked_by.retain(|&x| x != completed_id);
                    self.save(&task).ok();
                }
            }
        }
    }
}
```

**Rust 特性体现**：
- `filter_map` — 过滤 + 转换，一步完成
- `retain` — 就地过滤 `Vec`，保留满足条件的元素
- `contains` — 检查 `Vec` 是否包含某个值

### 6. 更新任务：update

```rust
pub fn update(&mut self, task_id: u32, status: Option<String>) -> String {
    let mut task = match self.load(task_id) {
        Ok(t) => t,
        Err(e) => return format!("Error: {}", e),
    };
    if let Some(s) = status {
        if !["pending", "in_progress", "completed"].contains(&s.as_str()) {
            return format!("Error: Invalid status: {}", s);
        }
        task.status = s.clone();
        if s == "completed" {
            self.clear_dependency(task_id);
        }
    }
    self.save(&task).ok();
    serde_json::to_string_pretty(&task).unwrap_or_default()
}
```

**Rust 特性体现**：
- `Option<String>` — 可选参数，`None` 表示不更新
- `if let Some(s) = status` — 模式匹配，只在有值时执行
- 提前返回 — 错误时直接 `return`，避免嵌套

### 7. 工具接入：agent_loop 中的 dispatch

```rust
match tool_name {
    // ...base tools...
    "task_create" => {
        let subject = args["subject"].as_str().unwrap_or("").to_string();
        let desc = args["description"].as_str().unwrap_or("").to_string();
        tasks.create(subject, desc)
    },
    "task_list" => tasks.list_all(),
    "task_get" => {
        let id = args["task_id"].as_u64().unwrap_or(0) as u32;
        tasks.get(id)
    },
    "task_update" => {
        let id = args["task_id"].as_u64().unwrap_or(0) as u32;
        let status = args["status"].as_str().map(|s| s.to_string());
        tasks.update(id, status)
    },
    _ => format!("Unknown tool: {}", tool_name),
}
```

**Rust 特性体现**：
- `as_str()` / `as_u64()` — JSON 值的类型转换
- `unwrap_or` — 提供默认值，避免 panic
- `map` — `Option` 的转换，`None` 保持 `None`

从 s07 起, 任务图是多步工作的默认选择。s03 的 Todo 仍可用于单次会话内的快速清单。

## 相对 s06 的变更

| 组件 | 之前 (s06) | 之后 (s07) |
|---|---|---|
| Tools | 5 | 8 (`task_create/update/list/get`) |
| 规划模型 | 扁平清单 (仅内存) | 带依赖关系的任务图 (磁盘) |
| 关系 | 无 | `blockedBy` 边 |
| 状态追踪 | 做完没做完 | `pending` -> `in_progress` -> `completed` |
| 持久化 | 压缩后丢失 | 压缩和重启后存活 |

## 试一试

```sh
cd learn-claude-code
python agents/s07_task_system.py
```

试试这些 prompt (英文 prompt 对 LLM 效果更好, 也可以用中文):

1. `Create 3 tasks: "Setup project", "Write code", "Write tests". Make them depend on each other in order.`
2. `List all tasks and show the dependency graph`
3. `Complete task 1 and then list tasks to see task 2 unblocked`
4. `Create a task board for refactoring: parse -> transform -> emit -> test, where transform and emit can run in parallel after parse`
