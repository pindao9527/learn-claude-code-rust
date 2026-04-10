# Day 04：优雅的分发 —— enum & match + 子智能体

对应原版：`s04_subagent.py`

## 今天做什么

在 `s03` 的基础上，移除 `TodoManager`，新增子智能体（Subagent）能力。

工作流程：
1. 复制 `s03_todo_write.rs`，重命名为 `s04_subagent.rs`
2. 删除所有 `TodoManager` 相关代码
3. 定义两套工具列表：`child_tools()` 和 `parent_tools()`
4. 定义两套 System Prompt：`SYSTEM` 和 `SUBAGENT_SYSTEM`
5. 实现 `run_subagent` 函数（独立上下文 + 安全轮次上限）
6. 在 `agent_loop` 的工具分发里新增 `"task"` 分支
7. 更新 `Cargo.toml`，注册 `s04` binary

---

## 学习目标

1. 理解"上下文隔离"的核心价值：子任务的中间噪声不污染父对话
2. 理解为什么父子智能体需要**不同的 System Prompt**
3. 理解为什么子智能体工具列表中**不能包含 `task`**（防无限递归）
4. 理解 Rust 中 async 函数的**递归限制**，以及如何通过内嵌循环规避

---

## 核心概念：上下文隔离

Python 版的关键只有一行：

```python
def run_subagent(prompt: str) -> str:
    sub_messages = [{"role": "user", "content": prompt}]  # 全新！
    ...
```

Rust 版的等价写法：

```rust
async fn run_subagent(prompt: &str, ...) -> String {
    let mut sub_messages: Vec<Message> = vec![
        Message::User { content: prompt.to_string() }  // 全新上下文
    ];
    ...
}
```

**不是共享父智能体的 `messages`，而是从一份新列表开始。** 这就是隔离的全部秘密。

---

## 两套工具：child_tools vs parent_tools

```
parent_tools = child_tools + [task]
                                ↑
                         子智能体不能有这个！
                         否则会无限递归派生子任务
```

Python 实现：
```python
CHILD_TOOLS  = [bash, read_file, write_file, edit_file]
PARENT_TOOLS = CHILD_TOOLS + [task]
```

Rust 实现方向：
```rust
fn child_tools() -> Value { json!([...]) }   // 4 个基础工具
fn parent_tools() -> Value { json!([...]) }  // 4 个 + task
```

---

## 两套 System Prompt

| | SYSTEM（父） | SUBAGENT_SYSTEM（子） |
|---|---|---|
| **职责** | 规划、外包子任务 | 执行子任务、总结结论 |
| **能力边界** | 可以说"这个交给子任务去做" | 只能做，不能再外包 |

Python 原版：
```python
SYSTEM = "You are a coding agent. Use the task tool to delegate..."
SUBAGENT_SYSTEM = "You are a coding subagent. Complete the given task, then summarize..."
```

---

## Rust 特有挑战：async 递归

如果 `agent_loop` 内调用 `run_subagent`，而 `run_subagent` 又调用 `agent_loop`，
会形成异步递归。Rust 编译器会报错：

```
error: recursive `async fn` requires boxing
```

**解决方案**：`run_subagent` **内嵌独立的 for 循环**，不复用 `agent_loop`。

```rust
async fn run_subagent(...) -> String {
    let mut sub_messages = vec![...];
    for _ in 0..30 {          // 不调用 agent_loop，自己写循环
        // 调用 LLM
        // 执行工具
        // if finish_reason != "tool_calls" { break }
    }
    // 只返回最后一条文本
}
```

代价：`run_subagent` 与 `agent_loop` 有少量重复代码。
收益：没有递归、没有 `Box::pin`，逻辑简单清晰。

---

## 今天要实现的结构

### run_subagent 签名

```rust
async fn run_subagent(
    prompt: &str,
    client: &Client,
    api_key: &str,
    base_url: &str,
    model_id: &str,
) -> String
```

### agent_loop 新增的 task 分支

```rust
"task" => {
    let desc = args["description"].as_str().unwrap_or("subtask");
    let prompt = args["prompt"].as_str().unwrap_or("");
    println!("\x1B[35m> task ({}): {}\x1B[0m", desc, &prompt[..prompt.len().min(80)]);
    run_subagent(prompt, client, api_key, base_url, model_id).await
}
```

---

## 思考题（写完后回答）

1. `run_subagent` 最后为什么要"丢弃"子上下文，只返回文本摘要？
2. 如果给子智能体保留 `task` 工具会发生什么？
3. 父智能体的 `messages` 里，`task` 工具的返回内容是什么形式？

---

## 完成标准

- `cargo check` 无报错
- `cargo run --bin s04` 能正常启动
- 输入要求使用子任务的问题，能看到 `> task (subtask): ...` 被打印
- 父对话只收到子智能体的一句话结论，不包含中间步骤
