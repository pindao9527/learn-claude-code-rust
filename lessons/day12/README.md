# Day 12：Worktree 任务隔离（Worktree + Task Isolation）

> *"各干各的目录，互不干扰。"* — 任务管目标，worktree 管目录，按 ID 绑定。

## 1. 目标

在 Day 11 中，Teammate 已经可以自主认领任务。但所有任务共享**同一个目录**。

想象两个 Teammate 同时重构不同模块：A 改 `config.rs`，B 也改 `config.rs`——未提交的变更互相污染，谁也没法干净回滚。

今天引入 **git worktree** 解决这个问题：给每个任务分配一个独立的隔离目录。

| 组件 | Day 11 | Day 12 |
|------|--------|--------|
| 执行范围 | 共享目录 | 每个任务独立 worktree |
| 任务追踪 | 只有 status/owner | 增加 `worktree` 字段绑定 |
| 可恢复性 | 仅任务状态 | 任务状态 + worktree 索引 |
| 收尾 | 任务完成 | keep（保留）或 remove（删除+完成） |
| 生命周期可见性 | 隐式日志 | `.worktrees/events.jsonl` 显式事件流 |

---

## 2. 架构：控制面 vs. 执行面

```
控制面 (.tasks/)                执行面 (.worktrees/)
+------------------+            +------------------------+
| task_1.json      |            | auth-refactor/         |
|  status: in_prog |<---------> |  branch: wt/auth-...   |
|  worktree: "..." |            |  task_id: 1            |
+------------------+            +------------------------+
| task_2.json      |            | ui-login/              |
|  status: pending |<---------> |  branch: wt/ui-login   |
+------------------+            +------------------------+
                                |
                      index.json (worktree 注册表)
                      events.jsonl (生命周期日志)

状态机：
  Task:     pending -> in_progress -> completed
  Worktree: absent  -> active      -> removed | kept
```

---

## 3. 三个新结构体

### 3.1 `EventBus` — 追加式生命周期事件

> **核心 Rust 主题**：`std::fs::OpenOptions`（追加写模式）

```rust
// Python 的 EventBus.emit() 用 open("a") 追加
// Rust 对应写法：
use std::fs::OpenOptions;
use std::io::Write;

fn emit(&self, event: &str, task_id: Option<u64>, worktree_name: Option<&str>) {
    let payload = json!({
        "event": event,
        "ts": now_secs(),
        "task": {"id": task_id},
        "worktree": {"name": worktree_name},
    });
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&self.path)
        .unwrap();
    writeln!(file, "{}", payload).unwrap();
}
```

事件类型清单：
- `worktree.create.before` / `worktree.create.after` / `worktree.create.failed`
- `worktree.remove.before` / `worktree.remove.after` / `worktree.remove.failed`
- `worktree.keep`
- `task.completed`

### 3.2 `TaskManager` — 带 worktree 绑定的任务板

相比 Day 11 的裸 JSON 文件操作，这次封装成结构体：

```rust
pub struct TaskManager {
    pub dir: PathBuf,
    next_id: Arc<Mutex<u64>>,  // 原子递增 ID
}

impl TaskManager {
    // 核心方法：
    pub fn create(&self, subject: &str, description: &str) -> String { ... }
    pub fn get(&self, task_id: u64) -> Result<Value, String> { ... }
    pub fn update(&self, task_id: u64, status: Option<&str>, owner: Option<&str>) -> String { ... }
    pub fn bind_worktree(&self, task_id: u64, worktree: &str) -> String { ... }
    pub fn unbind_worktree(&self, task_id: u64) -> String { ... }
    pub fn list_all(&self) -> String { ... }
}
```

关键点：`next_id` 用 `Arc<Mutex<u64>>` 而不是普通字段，因为 `TaskManager` 会被 `clone()` 传给多个线程（WorktreeManager 也需要引用它）。

### 3.3 `WorktreeManager` — 调用 git 子进程

> **核心 Rust 主题**：`std::process::Command`

```rust
pub struct WorktreeManager {
    pub repo_root: PathBuf,
    pub dir: PathBuf,        // .worktrees/
    pub index_path: PathBuf, // .worktrees/index.json
    pub tasks: TaskManager,
    pub events: EventBus,
    pub git_available: bool,
}
```

调用 git 的模板：

```rust
fn run_git(&self, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(&self.repo_root)  // 关键！在 repo 根目录执行
        .output()                       // 同步等待，捕获 stdout+stderr
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(msg);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

---

## 4. `std::process::Command` 详解

这是 Day 12 最重要的 Rust 知识点：

```rust
use std::process::Command;

// 方式一：output() —— 同步等待，获取全部输出（推荐用于 git）
let out = Command::new("git")
    .args(["worktree", "add", "-b", "wt/foo", ".worktrees/foo", "HEAD"])
    .current_dir("/path/to/repo")
    .output()?;  // 返回 Output { status, stdout, stderr }

// 方式二：spawn() —— 启动后台进程（Day 08 用过，适合长时间运行）
let mut child = Command::new("cargo")
    .arg("build")
    .spawn()?;
child.wait()?;

// 方式三：status() —— 只关心返回码
let ok = Command::new("git")
    .args(["rev-parse", "--is-inside-work-tree"])
    .status()?.success();
```

| 方法 | 是否等待 | 获取输出 | 用途 |
|------|----------|----------|------|
| `output()` | 是 | stdout + stderr | git 命令 ✅ |
| `status()` | 是 | 只有返回码 | 检查 git 是否可用 |
| `spawn()` | 否 | 需手动 wait | 后台长进程 |

---

## 5. 今日编码任务（手动实现顺序）

### 第一步：复用 s11 全部基础代码

原样复制（不改动）：
- `Message`、`InboxMessage`、`RequestStatus`、`ShutdownTracker`、`PlanTracker`
- `MessageBus`（含 `send_with_extra`、`broadcast`）
- `TeammateManager`
- `run_bash`、`safe_path`、`run_read`、`run_write`、`run_edit`
- `handle_shutdown_request`、`handle_plan_review`
- `make_identity_block`、`scan_unclaimed_tasks`、`claim_task`
- `_teammate_loop`（WORK + IDLE 全部逻辑）

新增导入：
```rust
use regex::Regex;  // 用于 worktree 名称校验（可选，也可用简单判断）
```

### 第二步：实现 `EventBus`

```rust
pub struct EventBus {
    pub path: PathBuf,
}

impl EventBus {
    pub fn new(path: PathBuf) -> Self { ... }

    // 追加一条 JSON 事件行
    pub fn emit(
        &self,
        event: &str,
        task: Option<serde_json::Value>,
        worktree: Option<serde_json::Value>,
        error: Option<&str>,
    ) { ... }

    // 读最近 N 条事件，返回 JSON 字符串
    pub fn list_recent(&self, limit: usize) -> String { ... }
}
```

提示：`list_recent` 读取全部行，取最后 `limit` 行，用 `serde_json::from_str` 逐行解析，解析失败时记录 `{"event": "parse_error", "raw": "..."}` 兜底。

### 第三步：实现 `TaskManager`

```rust
impl TaskManager {
    pub fn new(dir: PathBuf) -> Self {
        // 1. create_dir_all
        // 2. 扫描已有 task_*.json，找最大 ID，作为 next_id 初始值
    }

    fn path(&self, id: u64) -> PathBuf { self.dir.join(format!("task_{}.json", id)) }

    fn load(&self, id: u64) -> Result<Value, String> { ... }
    fn save(&self, task: &Value) -> Result<(), String> { ... }

    pub fn create(&self, subject: &str, description: &str) -> String {
        // 生成 ID（next_id 加一），构造 task JSON，写文件
    }

    pub fn bind_worktree(&self, task_id: u64, worktree: &str) -> String {
        // 加载 -> 设置 worktree 字段 -> 如果 status==pending 则改为 in_progress -> 保存
    }

    pub fn unbind_worktree(&self, task_id: u64) -> String {
        // 加载 -> 清空 worktree 字段 -> 保存
    }
}
```

### 第四步：实现 `WorktreeManager`

```rust
impl WorktreeManager {
    pub fn new(repo_root: PathBuf, tasks: TaskManager, events: EventBus) -> Self {
        // 1. 检测 git 是否可用（run git rev-parse）
        // 2. create_dir_all(.worktrees/)
        // 3. 如果 index.json 不存在，写入 {"worktrees": []}
    }

    fn run_git(&self, args: &[&str]) -> Result<String, String> { ... }

    fn load_index(&self) -> Value { ... }
    fn save_index(&self, data: &Value) { ... }
    fn find(&self, name: &str) -> Option<Value> { ... }

    pub fn create(&self, name: &str, task_id: Option<u64>, base_ref: &str) -> String {
        // 1. 校验名称（字母数字 . _ -，1-40字符）
        // 2. 检查 index 中是否已存在同名
        // 3. emit worktree.create.before
        // 4. run_git(["worktree", "add", "-b", branch, path, base_ref])
        // 5. 更新 index.json
        // 6. 如果有 task_id，调用 tasks.bind_worktree(task_id, name)
        // 7. emit worktree.create.after
        // 8. 失败时 emit worktree.create.failed
    }

    pub fn remove(&self, name: &str, force: bool, complete_task: bool) -> String {
        // 1. 查 index 找 worktree 条目
        // 2. emit worktree.remove.before
        // 3. run_git(["worktree", "remove", ...])
        // 4. 如果 complete_task && task_id 存在 -> tasks.update(completed) + tasks.unbind + emit task.completed
        // 5. 更新 index 中 status = "removed"
        // 6. emit worktree.remove.after
    }

    pub fn keep(&self, name: &str) -> String {
        // 更新 index status = "kept" + emit worktree.keep
    }

    pub fn run_in(&self, name: &str, command: &str) -> String {
        // 找到 worktree 路径 -> 用 Command::new("sh").arg("-c").arg(command).current_dir(path).output()
    }
}
```

### 第五步：扩充工具列表（`worktree_tools()`）

Lead 工具列表从 s11 的 `team_tools()` 扩充，新增：

```
task_create     task_list     task_get
task_update     task_bind_worktree
worktree_create worktree_list worktree_status
worktree_run    worktree_keep worktree_remove
worktree_events
```

### 第六步：主循环中连接新工具

在 Lead 的 `match tool_name { ... }` 分支中添加对应处理：

```rust
"task_create"       => tasks.create(subject, description),
"task_list"         => tasks.list_all(),
"task_get"          => tasks.get(task_id).unwrap_or_else(|e| e),
"task_update"       => tasks.update(task_id, status, owner),
"task_bind_worktree"=> tasks.bind_worktree(task_id, worktree),
"worktree_create"   => worktrees.create(name, task_id, base_ref),
"worktree_list"     => worktrees.list_all(),
"worktree_status"   => worktrees.status(name),
"worktree_run"      => worktrees.run_in(name, command),
"worktree_keep"     => worktrees.keep(name),
"worktree_remove"   => worktrees.remove(name, force, complete_task),
"worktree_events"   => events.list_recent(limit),
```

### 第七步：更新 main() 系统提示词

```rust
let system = format!(
    "You are a coding agent at {:?}. \
     Use task + worktree tools for multi-task work. \
     For parallel or risky changes: create tasks, allocate worktree lanes, \
     run commands in those lanes, then choose keep/remove for closeout.",
    workdir
);
```

---

## 6. Rust 知识点清单

| 概念 | 用途 | 本节示例 |
|------|------|---------|
| `std::process::Command` | 调用外部程序 | `git worktree add` |
| `.current_dir()` | 指定子进程工作目录 | 在 repo 根运行 git |
| `.output()` | 同步等待并捕获输出 | 获取 stdout/stderr |
| `output.status.success()` | 检查返回码 | 判断 git 是否成功 |
| `String::from_utf8_lossy()` | bytes → String | 解析 stdout/stderr |
| `OpenOptions::append(true)` | 追加写文件 | EventBus emit |
| `writeln!(file, ...)` | 写一行+换行 | JSONL 格式事件 |
| `Option<u64>` | 可选的 task_id | worktree 可以不绑定任务 |

---

## 7. 关键理解：为什么用 `git worktree`？

```
普通分支切换（git checkout）：
  Agent A 在 main 分支改了 src/lib.rs（未提交）
  Agent B 想切到 feature 分支……冲突！必须先 stash 或 commit

git worktree（本节方案）：
  Agent A 在 .worktrees/auth-refactor/ 修改文件
  Agent B 在 .worktrees/ui-login/ 修改文件
  两个目录完全独立，互不影响，可以同时运行 `git status`
```

**worktree 的本质**：同一个 git 仓库在文件系统上的多个检出（checkout），
共享 `.git` 对象库，但各有独立的 working tree 和 HEAD。

---

## 8. 测试提示词

程序运行后（`cargo run --bin s12`），依次尝试：

1. `Create tasks for backend auth and frontend login page, then list tasks.`
2. `Create worktree "auth-refactor" for task 1, then bind task 2 to a new worktree "ui-login".`
3. `Run "git status --short" in worktree "auth-refactor".`
4. `Keep worktree "ui-login", then list worktrees and inspect events.`
5. `Remove worktree "auth-refactor" with complete_task=true, then list tasks/worktrees/events.`
