# s02: Tool Use (工具使用)

`s00 > s01 > [ s02 ] > s03 > s04 > s05 > s06 > s07 > s08 > s09 > s10 > s11 > s12 > s13 > s14 > s15 > s16 > s17 > s18 > s19`

> *"加一个工具, 只加一个 handler"* -- 循环不用动, 新工具注册进 dispatch map 就行。
>
> **Harness 层**: 工具分发 -- 扩展模型能触达的边界。

## 问题

只有 `bash` 时, 所有操作都走 shell。`cat` 截断不可预测, `sed` 遇到特殊字符就崩, 每次 bash 调用都是不受约束的安全面。专用工具 (`read_file`, `write_file`) 可以在工具层面做路径沙箱。

关键洞察: 加工具不需要改循环。

## 解决方案

```
+--------+      +-------+      +------------------+
|  User  | ---> |  LLM  | ---> | Tool Dispatch    |
| prompt |      |       |      | {                |
+--------+      +---+---+      |   bash: run_bash |
                    ^           |   read: run_read |
                    |           |   write: run_wr  |
                    +-----------+   edit: run_edit |
                    tool_result | }                |
                                +------------------+

The dispatch map is a dict: {tool_name: handler_function}.
One lookup replaces any if/elif chain.
```

## 工作原理

1. 每个工具有一个处理函数。路径沙箱防止逃逸工作区。

```rust
fn safe_path(p: &str) -> Result<std::path::PathBuf, String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let path = cwd.join(p);
    if !path.starts_with(&cwd) {
        return Err(format!("Error: Path escapes workspace: {}", p));
    }
    Ok(path)
}

fn run_read(path_str: &str) -> String {
    let path = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return e,
    };
    std::fs::read_to_string(path).unwrap_or_else(|e| format!("Error: {}", e))
}
```

2. dispatch map 将工具名映射到处理函数。

```rust
// 在 Rust 中，通常不使用动态字面量字典，而是使用高效的 match 模式匹配：
let output = match tool_name {
    "bash" => run_bash(args["command"].as_str().unwrap_or("")),
    "read_file" => run_read(args["path"].as_str().unwrap_or("")),
    "write_file" => run_write(
        args["path"].as_str().unwrap_or(""), 
        args["content"].as_str().unwrap_or("")
    ),
    "edit_file" => run_edit(
        args["path"].as_str().unwrap_or(""),
        args["old_text"].as_str().unwrap_or(""),
        args["new_text"].as_str().unwrap_or("")
    ),
    _ => format!("Unknown tool: {}", tool_name),
};
```

3. 循环中按名称查找处理函数。循环体本身与 s01 完全一致。

```rust
if let Some(tool_calls) = choice["message"]["tool_calls"].as_array() {
    for tc in tool_calls {
        let tool_name = tc["function"]["name"].as_str().unwrap_or("");
        let args: Value = serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}")).unwrap();
        
        let output = match tool_name { ... }; // (详见上述匹配)
        
        results.push(Message::Tool {
            tool_call_id: tc["id"].as_str().unwrap_or("").to_string(),
            content: output,
        });
    }
}
```

加工具 = 加 handler + 加 schema。循环永远不变。

## 相对 s01 的变更

| 组件           | 之前 (s01)         | 之后 (s02)                     |
|----------------|--------------------|--------------------------------|
| Tools          | 1 (仅 bash)        | 4 (bash, read, write, edit)    |
| Dispatch       | 硬编码 bash 调用   | `TOOL_HANDLERS` 字典           |
| 路径安全       | 无                 | `safe_path()` 沙箱             |
| Agent loop     | 不变               | 不变                           |

## 试一试

```sh
cd learn-claude-code-rust
cargo run --bin s02
```

试试这些 prompt (英文 prompt 对 LLM 效果更好, 也可以用中文):

1. `Read the file requirements.txt`
2. `Create a file called greet.py with a greet(name) function`
3. `Edit greet.py to add a docstring to the function`
4. `Read greet.py to verify the edit worked`

## 如果你开始觉得“工具不只是 handler map”

到这里为止，教学主线先把工具讲成：

- schema
- handler
- `tool_result`

这是对的，而且必须先这么学。

但如果你继续把系统做大，很快就会发现工具层还会继续长出：

- 权限环境
- 当前消息和 app state
- MCP client
- 文件读取缓存
- 通知与 query 跟踪

也就是说，在一个结构更完整的系统里，工具层最后会更像一条“工具控制平面”，而不只是一张分发表。

这层不要抢正文主线。  
你先把这一章吃透，再继续看：

- [`s02a-tool-control-plane.md`](./s02a-tool-control-plane.md)

## 消息规范化

教学版的 `messages` 列表直接发给 API, 所见即所发。但当系统变复杂后 (工具超时、用户取消、压缩替换), 内部消息列表会出现 API 不接受的格式问题。需要在发送前做一次规范化。

### 为什么需要

API 协议有三条硬性约束:
1. 每个 `tool_use` 块**必须**有匹配的 `tool_result` (通过 `tool_use_id` 关联)
2. `user` / `assistant` 消息必须**严格交替** (不能连续两条同角色)
3. 只接受协议定义的字段 (内部元数据会导致 400 错误)

### 实现

```rust
fn normalize_messages(messages: &[Message]) -> Vec<Message> {
    // 将内部消息列表规范化为 API 可接受的格式。
    let mut normalized = Vec::new();
    let mut existing_results = std::collections::HashSet::new();

    // Step 1: 剥离内部字段，记录已存在的 Tool 响应 ID
    for msg in messages {
        match msg {
            Message::Tool { tool_call_id, .. } => {
                existing_results.insert(tool_call_id.clone());
            }
            _ => { /* 剥离多余数据... */ }
        }
    }

    // Step 2: 填充缺失配对的 tool_use, 插入占位结果
    // Step 3: 合并连续的同角色 Message (例如连续发送两个 User 提示)
    // （在 Rust 中这里将以迭代、状态机、模式匹配形式被大量简化或显式编写）
    // ...

    normalized
}
```

在 agent loop 中, 每次 API 调用前运行:

```rust
let body = json!({
    "model": model_id,
    "system": system,
    "messages": normalize_messages(&messages), // 规范化后再发送
    "tools": TOOLS,
    "max_tokens": 8000
});

// 发起 HTTP Client reqwest 请求 ...
```

**关键洞察**: `messages` 列表是系统的内部表示, API 看到的是规范化后的副本。两者不是同一个东西。

## 教学边界

这一章最重要的，不是把完整工具运行时一次讲全，而是先讲清 3 个稳定点：

- tool schema 是给模型看的说明
- handler map 是代码里的分发入口
- `tool_result` 是结果回流到主循环的统一出口

只要这三点稳住，读者就已经能自己在不改主循环的前提下新增工具。

权限、hook、并发、流式执行、外部工具来源这些后续层次当然重要，但都应该建立在这层最小分发模型之后。
