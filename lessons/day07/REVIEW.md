# Day 07 复习卡：任务系统与文件持久化

## 1. 核心理论：状态在对话之外存活

s03 的 `TodoManager` 只活在内存里，上下文压缩一跑就没了。
s07 的 `TaskManager` 把每个任务写成一个 JSON 文件，重启、压缩都不会丢失。

这是一个根本性的转变：**把 Agent 的工作状态从"对话内"移到"文件系统上"**。

## 2. serde 派生：让结构体自动会读写 JSON

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

`#[derive(...)]` 让编译器自动生成代码：

- `Serialize` — 能把 `Task` 转成 JSON 字符串（写文件）
- `Deserialize` — 能把 JSON 字符串还原成 `Task`（读文件）
- `Debug` — 能用 `{:?}` 打印
- `Clone` — 能 `.clone()` 复制

不需要手写任何序列化逻辑，编译器全包了。

## 3. Option 链式调用：安全解析文件名

从 `"task_3.json"` 中提取数字 `3`，用链式 `and_then`：

```rust
s.strip_prefix("task_")
    .and_then(|s| s.strip_suffix(".json"))
    .and_then(|s| s.parse::<u32>().ok())
```

任何一步失败（文件名不匹配、解析失败）都会短路返回 `None`，不会 panic。
这是 Rust 处理"可能失败的多步操作"的惯用法。

## 4. retain：就地过滤 Vec

依赖解除的核心一行：

```rust
task.blocked_by.retain(|&x| x != completed_id);
```

`retain` 就地删除不满足条件的元素，等价于 Python 的：

```python
task["blockedBy"] = [x for x in task["blockedBy"] if x != completed_id]
```

区别是 Rust 不分配新内存，直接在原 `Vec` 上操作。

## 5. &mut self vs &self

今天遇到了两种方法签名：

- `fn create(&mut self, ...)` — 需要修改 `next_id`，必须可变借用
- `fn load(&self, ...)` — 只读，不可变借用就够了
- `fn save(&self, ...)` — 只写文件，不修改结构体字段，不可变借用

Rust 编译器会在你写错时报错，这是所有权系统保护你不犯错的方式。

## 6. map_err：错误类型转换

```rust
std::fs::read_to_string(&path)
    .map_err(|_| format!("Task {} not found", task_id))?;
```

`read_to_string` 返回 `io::Error`，但我们的函数返回 `String` 错误。
`map_err` 把错误类型转换，`?` 再把错误提前返回。

---

### 今天的收获

文件 I/O 在 Rust 里比想象中简单——`serde` 处理序列化，`std::fs` 处理读写，`PathBuf::join` 处理路径拼接。真正需要思考的是**错误处理**：每一步都可能失败，Rust 强迫你把每种失败都想清楚。
