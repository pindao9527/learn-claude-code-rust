# Day 02: 所有权、借用与多工具分发 (Ownership & Tool Dispatch)

## 核心概念：为什么 Rust 总是让你 `.clone()`?

在 Day 01 的实现中，你可能会注意到我们在构建消息体时频繁调用了 `messages.clone()`。这是因为 Rust 的**所有权 (Ownership)** 规则：

1. **移动 (Move)**: 当你把一个 `Vec` 传给 `json!` 宏或函数时，如果没有明确借用，它的所有权就被"移走"了。
2. **克隆 (Clone)**: 为了保留原始数据并在下一轮循环中使用，我们不得不进行内存复制（Clone）。

Day 02 的解法是用 `messages.iter()` 遍历——只借用，不移动，彻底消除多余的 clone。

## 为什么用 `enum Message` 而不是 `struct`？

这是 Day 02 最核心的设计决策。

`s01` 里消息用的是 `serde_json::Value`，完全没有类型约束——你可以往里塞任何东西，编译器不会报错，但运行时可能崩。

`s02` 改用了 `enum Message`：

```rust
enum Message {
    System { content: String },
    User { content: String },
    Assistant { content: Option<String>, tool_calls: Option<Vec<Value>> },
    Tool { content: String, tool_call_id: String },
}
```

**为什么是 `enum` 而不是 `struct`？**

因为一条消息"只能是其中一种"——它要么是用户消息，要么是助手消息，不可能同时是两种。这正是 `enum` 的语义：**互斥的多种可能**。

- `struct` 描述的是"它同时拥有这些字段"
- `enum` 描述的是"它是这些形态之一"

**实际收益**：

- `Assistant` 有 `tool_calls` 字段，`User` 没有——编译器强制保证你不会在用户消息上访问 `tool_calls`。
- `#[serde(tag = "role")]` 让序列化自动加上 `"role": "user"` 等字段，完美对齐 OpenAI API 协议，零手写。

## 工具扩展 (Tool Expansion)

我们将复刻 `s02_tool_use.py` 中的功能，增加以下工具：

- `read_file`: 读取文件内容。
- `write_file`: 写入新文件（支持递归创建目录）。
- `edit_file`: 替换文件中的特定文本段落。

---

> **思考题**：在 `agent_loop(&mut Vec<Message>)` 中，如果我们在循环内部直接把 `messages` 传给某个会转移所有权的函数，循环的下一轮还能访问它吗？这就是为什么我们要学习"借用"。
