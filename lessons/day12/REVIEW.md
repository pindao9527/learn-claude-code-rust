# Day 12: Review & Key Concepts

## 1. `std::process::Command`：Rust 调用外部程序

Day 12 最核心的新 Rust 主题。有三种主要用法：

```rust
use std::process::Command;

// 用法一：output() —— 同步等待，捕获 stdout + stderr（最常用）
let output = Command::new("git")
    .args(["worktree", "add", "-b", "wt/foo", ".worktrees/foo", "HEAD"])
    .current_dir(&repo_root)   // 指定工作目录
    .output()                   // 阻塞等待完成
    .map_err(|e| e.to_string())?;

// 检查结果
if !output.status.success() {
    let err = String::from_utf8_lossy(&output.stderr);
    return Err(format!("git failed: {}", err));
}
let stdout = String::from_utf8_lossy(&output.stdout);

// 用法二：status() —— 只关心返回码
let ok = Command::new("git")
    .args(["rev-parse", "--is-inside-work-tree"])
    .current_dir(&dir)
    .status()
    .map(|s| s.success())
    .unwrap_or(false);

// 用法三：spawn() —— 不等待，返回子进程句柄（Day 08 用过）
let mut child = Command::new("cargo").arg("build").spawn()?;
child.wait()?;
```

| 方法 | 等待完成 | 获取输出 | 用途 |
|------|----------|----------|------|
| `output()` | ✅ | stdout + stderr | git 命令 ✅ |
| `status()` | ✅ | 只有返回码 | 检测工具是否存在 |
| `spawn()` | ❌ | 需手动 wait | 后台长进程 |

**为什么用 `.current_dir()`？**  
`git worktree` 命令必须在 repo 根目录下执行，否则 git 找不到 `.git/`。`current_dir()` 指定子进程的工作目录，而非 Rust 程序自身的 `env::current_dir()`。

---

## 2. `OpenOptions` 追加写模式：EventBus 的核心

```rust
use std::fs::OpenOptions;
use std::io::Write;

// 追加写（append = true）：文件内容不会被清空
let mut file = OpenOptions::new()
    .append(true)   // 追加到末尾
    .create(true)   // 文件不存在时自动创建
    .open(&path)?;

writeln!(file, "{}", json_line)?;  // 写一行 + 换行符

// 对比：普通写（覆盖）
// fs::write(&path, content)?;  ← 清空并重写
```

JSONL（JSON Lines）格式的核心正是「每行一个 JSON 对象」+「只追加，不重写」。
这保证了崩溃后不会丢失之前的事件记录。

---

## 3. 任务板与 worktree 的双向绑定

```
创建任务 → task_1.json { status: "pending", worktree: "" }
           ↓
创建 worktree (task_id=1)
  → git worktree add -b wt/auth .worktrees/auth HEAD
  → index.json 新增条目 { name: "auth", task_id: 1 }
  → task_1.json 更新 { worktree: "auth", status: "in_progress" }
           ↓
在 worktree 中执行命令
  → Command::new("sh").arg("-c").arg(cmd).current_dir(".worktrees/auth")
           ↓
收尾（两种选择）：
  keep:   index.json { status: "kept" } + emit worktree.keep
  remove: git worktree remove .worktrees/auth
          → task_1.json { status: "completed", worktree: "" }
          → index.json { status: "removed" }
          → emit task.completed
```

`bind_worktree` 同时写两侧（任务 + worktree 索引），是「任务板↔执行面」的桥接点。

---

## 4. `Option<u64>` 作为可选 task_id

`worktree_create` 的 `task_id` 是**可选的**——worktree 可以不绑定任何任务：

```rust
pub fn create(&self, name: &str, task_id: Option<u64>, base_ref: &str) -> String {
    // ...git 操作...
    
    // 只有 task_id 存在时才绑定
    if let Some(id) = task_id {
        self.tasks.bind_worktree(id, name);
    }
    
    // index.json 条目中 task_id 字段为 null 或具体 ID
    let entry = json!({
        "name": name,
        "task_id": task_id,  // Option<u64> 序列化：Some(1) -> 1，None -> null
        "status": "active",
    });
}
```

`serde_json` 会自动把 `Option<u64>` 的 `None` 序列化为 JSON `null`，`Some(1)` 序列化为 `1`。

---

## 5. `next_id` 的线程安全设计

Python 的 `TaskManager` 用实例变量 `self._next_id += 1`，单线程没问题。

Rust 中 `TaskManager` 会被 clone 到多个线程（WorktreeManager 持有引用），所以：

```rust
pub struct TaskManager {
    pub dir: PathBuf,
    next_id: Arc<Mutex<u64>>,  // 不能是普通 u64 字段！
}

impl TaskManager {
    pub fn create(&self, subject: &str, description: &str) -> String {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        // 用 id 创建任务文件...
    }
}
```

**更简单的替代方案**：每次 `create` 时重新扫描目录找最大 ID，用 `max_id + 1`。
这样不需要 `Arc<Mutex<u64>>`，但每次创建任务都要遍历目录（性能略低）。
对于学习项目，这个简化是可以接受的。

---

## 6. 状态机：Worktree 生命周期

```
absent → [create] → active → [keep]   → kept
                           → [remove] → removed
```

和 Task 状态机配合：

```
Task:     pending → [bind_worktree] → in_progress → [remove+complete] → completed
Worktree: absent  → [create]        → active       → [remove]          → removed
```

两个状态机通过 `task_id` 字段关联。`WorktreeManager.remove()` 的 `complete_task: bool` 参数让一次调用同时推进两个状态机。

---

## 7. 崩溃恢复（Crash Recovery）

Day 12 的设计使崩溃后可以恢复：

```
内存（易失）：                磁盘（持久）：
  - 变量 messages              .tasks/task_N.json  ← 任务状态
  - 变量 worktrees             .worktrees/index.json ← worktree 注册表
  ← 崩溃后全部丢失            .worktrees/events.jsonl ← 完整事件历史
                              .worktrees/auth-refactor/ ← 实际文件
```

重启后：
1. 读 `.tasks/` 恢复所有任务状态
2. 读 `.worktrees/index.json` 知道哪些 worktree 还存在
3. 读 `events.jsonl` 还原完整的操作历史

这就是**磁盘状态 > 内存状态**的设计哲学——也是 Day 07 任务系统的核心思想在多进程场景下的延伸。

---

## 💡 课后挑战

1. **`worktree_status` 工具**：在 `run_in` 的基础上，专门实现 `git status --short --branch` 的快捷方式，让 AI 能快速查看 worktree 的干净程度。

2. **孤立 worktree 检测**：实现一个函数，对比 `index.json` 中的条目和 `git worktree list --porcelain` 的输出，找出「在磁盘上存在但不在 index 中」或「在 index 中但已从磁盘消失」的孤立 worktree。

3. **思考题**：为什么 `WorktreeManager` 不需要 `Arc<Mutex<>>` 包裹 index.json？
   （提示：s12 的主循环是单线程的——只有 Lead 操作 worktree，Teammate 只操作任务板。和 s11 的 `TeammateManager` 对比，谁需要锁，谁不需要？）
