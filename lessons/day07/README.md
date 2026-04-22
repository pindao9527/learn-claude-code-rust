# Day 07：任务系统（Task System / DAG）

对应原版：`s07_task_system.py`

## 今天做什么

到了 `s06`，agent 已经能压缩上下文、无限运行了。但有一个问题：

**对话结束后，agent 做过什么就全忘了。**

`s07` 引入任务系统解决这个问题——把任务持久化为 `.tasks/` 目录下的 JSON 文件，任务状态在对话之间存活。

```
.tasks/
  task_1.json  {"id":1, "subject":"...", "status":"completed", ...}
  task_2.json  {"id":2, "blockedBy":[1], "status":"pending", ...}
  task_3.json  {"id":3, "blockedBy":[2], ...}
```

依赖关系（DAG）：

```
task_1 完成 --> 从 task_2 的 blockedBy 中移除 --> task_2 解锁
```

**核心洞察："状态在压缩之外存活——因为它在对话之外。"**

---

## Rust 学习重点：文件 I/O + serde_json 进阶

今天的新知识集中在两件事：

### 1. PathBuf 路径操作

```rust
let path = self.dir.join(format!("task_{}.json", id));
```

- `PathBuf` 是 Rust 的拥有所有权的路径类型
- `join()` 拼接路径，跨平台安全
- 和 Python 的 `Path / "task_1.json"` 等价

### 2. serde_json 读写结构体

```rust
// 序列化：Task -> JSON 字符串
let json = serde_json::to_string_pretty(&task)?;
std::fs::write(&path, json)?;

// 反序列化：JSON 字符串 -> Task
let content = std::fs::read_to_string(&path)?;
let task: Task = serde_json::from_str(&content)?;
```

关键：`Task` 结构体需要派生 `Serialize` 和 `Deserialize`：

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Task {
    pub id: u32,
    pub subject: String,
    pub status: String,
    pub blocked_by: Vec<u32>,
    pub owner: String,
}
```

### 3. read_dir 遍历目录

```rust
for entry in std::fs::read_dir(&self.dir)? {
    let entry = entry?;
    let file_name = entry.file_name();
    let name = file_name.to_string_lossy();
    // 解析 "task_3.json" 中的数字 3
}
```

---

## 需要新增的内容（在 s06 基础上）

从 `s06_context_compact.rs` 复制后，只需要新增：

1. **`Task` 结构体**：任务的数据模型
2. **`TaskManager` 结构体 + impl**：CRUD + 依赖清除
3. **4 个新工具**：`task_create` / `task_list` / `task_get` / `task_update`
4. **`agent_loop` 里加 dispatch**：4 个新工具的 match 分支
5. **`main` 里初始化 `TaskManager`**

其余代码（`Message`、`run_bash`、压缩器、`agent_loop` 骨架）完全不动。

---

## 思考题（写完后回答）

1. `Task` 的 `status` 字段用 `String` 还是用 `enum` 更符合 Rust 惯用法？各有什么取舍？
2. `_clear_dependency` 需要遍历所有任务文件，如果有 10000 个任务会怎样？有什么优化思路？
3. 为什么 `next_id` 不能直接从 `0` 开始，而要先扫描目录计算 `max_id`？

---

## 完成标准

- `cargo build --bin s07` 编译通过
- 能创建任务、列出任务、更新状态
- 完成一个任务后，依赖它的任务自动从 `blockedBy` 中移除
- `.tasks/` 目录下能看到持久化的 JSON 文件
