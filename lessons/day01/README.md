# Day 01: 基础骨架与 Rust 初印象

## 1. 核心语法点

### 变量与可变性 (Mutability)

- **默认不可变**: `let api_key = ...`
- **显式可变**: `let mut history = vec![];`
- *心得*: Rust 的默认不可变机制减少了意外修改数据的 BUG。

### 控制流 (Control Flow)

- **无限循环**: `loop { ... }` 用于 REPL 和 Agent 轮询。
- **模式匹配入门**:
  - `if let Some(val) = option`：优雅地处理"可能为空"的情况。
  - `match result { ... }`：强制你处理所有可能的错误分支。

### 宏 (Macros)

- `json!`: 像写 Python 字典一样写 JSON。
- `vec!`: 列表初始化。
- `println!`: 格式化输出。

## 2. 异步编程 (Async)

- 使用 `#[tokio::main]` 标识程序的异步入口。
- `await?` 是处理网络请求的"标配"：等待结果，如果有错就地向上抛出。

## 3. 我的第一个 Agent 循环

在 `s01_agent_loop.rs` 中，我实现了一个基本的"思考-执行"循环：

1. 用户输入 query。
2. 调用 LLM 并提供 `bash` 函数工具。
3. 如果 LLM 决定调用工具，执行 `std::process::Command` 并把结果塞回消息历史。
4. 重复上述过程，直到 LLM 给出最终回复。

## 4. 为什么要 `.clone()`？

在 `agent_loop` 里有这样一行：

```rust
full_messages.extend(messages.clone());
```

为什么不直接 `extend(messages)`？

因为 `extend` 会把 `messages` 的所有权"吃掉"——下一轮循环就再也访问不到它了。
`.clone()` 是一个临时解法：在堆上复制一份数据，让 `messages` 的原始所有权保持不变。

**代价**：每次 LLM 调用都会把整个历史记录复制一遍，历史越长，浪费越多。

Day 02 会学习用**借用（`&`）+ 迭代器**彻底消除这个不必要的 clone。

---

### 思考题预览 (Day 02 预热)

在 `agent_loop` 函数中，参数定义是 `messages: &mut Vec<Value>`。

- `&`：表示"借用"，我把数据借给函数用，但不交出地契（所有权）。
- `mut`：表示"允许修改"，函数拿到的是"可变借用"。
- **为什么要借？** 如果不写 `&`，地契就给了函数，函数执行完后，我的 `history` 就会被销毁，主循环就没法继续对话了。
