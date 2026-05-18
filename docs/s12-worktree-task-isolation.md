# s12: Worktree + Task Isolation (Worktree 任务隔离)

`s01 > s02 > s03 > s04 > s05 > s06 | s07 > s08 > s09 > s10 > s11 > [ s12 ]`

> *"各干各的目录, 互不干扰"* -- 任务管目标, worktree 管目录, 按 ID 绑定。
>
> **Harness 层**: 目录隔离 -- 永不碰撞的并行执行通道。

## 问题

到 s11, Agent 已经能自主认领和完成任务。但所有任务共享一个目录。两个 Agent 同时重构不同模块 -- A 改 `config.rs`, B 也改 `config.rs`, 未提交的改动互相污染, 谁也没法干净回滚。

任务板管 "做什么" 但不管 "在哪做"。解法: 给每个任务一个独立的 git worktree 目录, 用任务 ID 把两边关联起来。

## 解决方案

```
Control plane (.tasks/)             Execution plane (.worktrees/)
+------------------+                +------------------------+
| task_1.json      |                | auth-refactor/         |
|   status: in_progress  <------>   branch: wt/auth-refactor
|   worktree: "auth-refactor"   |   task_id: 1             |
+------------------+                +------------------------+
| task_2.json      |                | ui-login/              |
|   status: pending    <------>     branch: wt/ui-login
|   worktree: "ui-login"       |   task_id: 2             |
+------------------+                +------------------------+
                                    |
                          index.json (worktree registry)
                          events.jsonl (lifecycle log)

State machines:
  Task:     pending -> in_progress -> completed
  Worktree: absent  -> active      -> removed | kept
```

## 工作原理

1. **创建任务。** 先把目标持久化。

```rust
tasks.create("Implement auth refactor", "");
// -> .tasks/task_1.json  status=pending  worktree=""
```

2. **创建 worktree 并绑定任务。** 传入 `task_id` 自动将任务推进到 `in_progress`。

```rust
worktrees.create("auth-refactor", Some(1), "HEAD");
// -> git worktree add -b wt/auth-refactor .worktrees/auth-refactor HEAD
// -> index.json gets new entry, task_1.json gets worktree="auth-refactor"
```

绑定同时写入两侧状态（Rust 实现）：

```rust
pub fn bind_worktree(&self, task_id: u64, worktree: &str) -> String {
    let mut task = match self.load(task_id) {
        Ok(t) => t,
        Err(e) => return e,
    };
    task["worktree"] = json!(worktree);
    if task["status"].as_str() == Some("pending") {
        task["status"] = json!("in_progress");
    }
    task["updated_at"] = json!(now_secs());
    self.save(&task).unwrap_or_default();
    serde_json::to_string_pretty(&task).unwrap_or_default()
}
```

3. **在 worktree 中执行命令。** `current_dir` 指向隔离目录。

```rust
pub fn run_in(&self, name: &str, command: &str) -> String {
    let wt = match self.find(name) {
        Some(w) => w,
        None => return format!("Error: Unknown worktree '{}'", name),
    };
    let path = PathBuf::from(wt["path"].as_str().unwrap_or(""));
    if !path.exists() {
        return format!("Error: Worktree path missing: {:?}", path);
    }

    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&path)          // 关键：在隔离目录中执行
        .output();

    match output {
        Ok(out) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            let trimmed = combined.trim().to_string();
            if trimmed.is_empty() { "(no output)".to_string() } else { trimmed }
        }
        Err(e) => format!("Error: {}", e),
    }
}
```

4. **收尾。** 两种选择:
   - `worktrees.keep(name)` -- 保留目录供后续使用。
   - `worktrees.remove(name, false, true)` -- 删除目录, 完成绑定任务, 发出事件。一个调用搞定拆除 + 完成。

```rust
pub fn remove(&self, name: &str, force: bool, complete_task: bool) -> String {
    let wt = match self.find(name) {
        Some(w) => w,
        None => return format!("Error: Unknown worktree '{}'", name),
    };

    self.events.emit("worktree.remove.before", /* ... */);

    let mut args = vec!["worktree", "remove"];
    if force { args.push("--force"); }
    let path_str = wt["path"].as_str().unwrap_or("");
    args.push(path_str);

    match self.run_git(&args) {
        Err(e) => {
            self.events.emit("worktree.remove.failed", /* error=e */);
            return format!("Error: {}", e);
        }
        Ok(_) => {}
    }

    if complete_task {
        if let Some(task_id) = wt["task_id"].as_u64() {
            self.tasks.update(task_id, Some("completed"), None);
            self.tasks.unbind_worktree(task_id);
            self.events.emit("task.completed", /* task_id */);
        }
    }

    // 更新 index.json 中的状态
    let mut idx = self.load_index();
    for item in idx["worktrees"].as_array_mut().unwrap_or(&mut vec![]) {
        if item["name"].as_str() == Some(name) {
            item["status"] = json!("removed");
            item["removed_at"] = json!(now_secs());
        }
    }
    self.save_index(&idx);
    self.events.emit("worktree.remove.after", /* ... */);
    format!("Removed worktree '{}'", name)
}
```

5. **事件流。** 每个生命周期步骤追加写入 `.worktrees/events.jsonl`：

```rust
pub struct EventBus {
    pub path: PathBuf,
}

impl EventBus {
    pub fn emit(
        &self,
        event: &str,
        task: Option<serde_json::Value>,
        worktree: Option<serde_json::Value>,
        error: Option<&str>,
    ) {
        let mut payload = json!({
            "event": event,
            "ts": now_secs(),
            "task": task.unwrap_or(json!({})),
            "worktree": worktree.unwrap_or(json!({})),
        });
        if let Some(e) = error {
            payload["error"] = json!(e);
        }

        // OpenOptions::append(true) 保证追加写入，不覆盖历史
        if let Ok(mut file) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{}", payload);
        }
    }
}
```

事件样例（写入 `.worktrees/events.jsonl`）：

```json
{"event":"worktree.remove.after","task":{"id":1,"status":"completed"},"worktree":{"name":"auth-refactor","status":"removed"},"ts":1730000000}
```

事件类型: `worktree.create.before/after/failed`, `worktree.remove.before/after/failed`, `worktree.keep`, `task.completed`。

崩溃后从 `.tasks/` + `.worktrees/index.json` 重建现场。会话记忆是易失的; 磁盘状态是持久的。

## `std::process::Command`：调用 git 子进程

这是 s12 新增的核心 Rust 主题：

```rust
fn run_git(&self, args: &[&str]) -> Result<String, String> {
    if !self.git_available {
        return Err("Not in a git repository. worktree tools require git.".to_string());
    }

    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(&self.repo_root)   // 在 repo 根目录执行
        .output()                        // 同步等待，捕获 stdout+stderr
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let msg = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return Err(msg.trim().to_string());
    }
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(if out.trim().is_empty() { "(no output)".to_string() } else { out.trim().to_string() })
}
```

检测 git 是否可用：

```rust
fn is_git_repo(dir: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

## 相对 s11 的变更

| 组件               | 之前 (s11)                 | 之后 (s12)                                   |
|--------------------|----------------------------|----------------------------------------------|
| 协调               | 任务板 (owner/status)      | 任务板 + worktree 显式绑定                   |
| 执行范围           | 共享目录                   | 每个任务独立目录                             |
| 可恢复性           | 仅任务状态                 | 任务状态 + worktree 索引                     |
| 收尾               | 任务完成                   | 任务完成 + 显式 keep/remove                  |
| 生命周期可见性     | 隐式日志                   | `.worktrees/events.jsonl` 显式事件流         |
| 子进程             | `run_bash` (sh -c)         | `run_git` (std::process::Command)            |

## 新增工具列表

s12 在 s11 全部工具基础上新增：

| 工具 | 功能 |
|------|------|
| `task_create` | 创建任务到 `.tasks/task_N.json` |
| `task_list` | 列出所有任务（含 worktree 绑定信息）|
| `task_get` | 按 ID 查看任务详情 |
| `task_update` | 更新任务 status / owner |
| `task_bind_worktree` | 手动绑定任务到 worktree |
| `worktree_create` | `git worktree add` + 注册 + 可选绑定任务 |
| `worktree_list` | 列出 index.json 中的所有 worktree |
| `worktree_status` | `git status --short --branch` in worktree |
| `worktree_run` | 在指定 worktree 目录中执行 shell 命令 |
| `worktree_keep` | 标记 worktree 为 kept（不删除）|
| `worktree_remove` | 删除 worktree，可选完成绑定任务 |
| `worktree_events` | 查看最近 N 条生命周期事件 |

## 试一试

```sh
cd learn-claude-code-rust
cargo run --bin s12
```

试试这些 prompt (英文 prompt 对 LLM 效果更好, 也可以用中文):

1. `Create tasks for backend auth and frontend login page, then list tasks.`
2. `Create worktree "auth-refactor" for task 1, then bind task 2 to a new worktree "ui-login".`
3. `Run "git status --short" in worktree "auth-refactor".`
4. `Keep worktree "ui-login", then list worktrees and inspect events.`
5. `Remove worktree "auth-refactor" with complete_task=true, then list tasks/worktrees/events.`
