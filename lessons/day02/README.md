# Day 02: 所有权、借用与多工具分发 (Ownership & Tool Dispatch)

## 核心概念：为什么 Rust 总是让你 `.clone()`?

在 Day 01 的实现中，你可能会注意到我们在构建消息体时频繁调用了 `messages.clone()`。这是因为 Rust 的**所有权 (Ownership)** 规则：

1.  **移动 (Move)**: 当你把一个 `Vec` 传给 `json!` 宏或函数时，如果没有明确借用，它的所有权就被“移走”了。
2.  **克隆 (Clone)**: 为了保留原始数据并在下一轮循环中使用，我们不得不进行内存复制（Clone）。

### 在 Day 02 中，我们将学习：
- **借用 (Borrowing)**: 使用 `&Vec<T>` 或 `&[T]` (切片) 来只读访问数据。
- **结构体与枚举 (Enum)**: 使用强类型的结构体替代 `serde_json::Value`，让你的代码在编译期就能发现 JSON 字段写错的问题。

## 工具扩展 (Tool Expansion)

我们将复刻 `s02_tool_use.py` 中的功能，增加以下工具：
- `read_file`: 读取文件内容。
- `write_file`: 写入新文件（支持递归创建目录）。
- `edit_file`: 替换文件中的特定文本段落。

---
> **思考题**：在 `agent_loop(&mut Vec<Message>)` 中，如果我们在循环内部直接把 `messages` 传给某个会转移所有权的函数，循环的下一轮还能访问它吗？这就是为什么我们要学习“借用”。
