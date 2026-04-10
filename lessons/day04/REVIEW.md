# Day 04 复习卡：子智能体 & 上下文隔离

## 1. 核心原理只有一句话

**子智能体的价值不是"多一个模型"，而是"多一个干净上下文"。**

```
父上下文（干净）           子上下文（用完即丢）
─────────────             ─────────────────────
messages: [               sub_messages: [
  User: "..."               User: "调查.rs文件"
  Assistant: tool_call      Assistant: tool_call
  Tool: "共有8个.rs"  ←     Tool: "8"         ]
  Assistant: "共有8个"  ]   ↑
                            只把这行带回来
```

## 2. 两套工具定义

```rust
fn child_tools() -> Value  { json!([bash, read, write, edit])       }
fn parent_tools() -> Value { json!([bash, read, write, edit, task]) }
//                                                           ↑
//                              子智能体永远不能有这个！否则无限递归
```

## 3. run_subagent 的骨架

```rust
async fn run_subagent(prompt: &str, ...) -> String {
    let mut sub_messages = vec![Message::User { content: prompt.to_string() }];
    //                          ↑ 全新上下文，不继承父对话

    for _ in 0..30 {  // 安全轮次上限
        let resp = match client.post(...)
            .json(&json!({ "tools": child_tools(), ... }))  // 用子工具集
            .send().await {
            Ok(r) => match r.json::<Value>().await { Ok(v) => v, Err(_) => break },
            Err(_) => break,
        };
        // ...执行工具，push results 进 sub_messages...
        if finish_reason != "tool_calls" { break; }
    }

    // 只返回最后一条文本，sub_messages 全部丢弃
    if let Some(Message::Assistant { content: Some(text), .. }) = sub_messages.last() {
        text.clone()
    } else {
        "(no summary)".to_string()
    }
}
```

## 4. Rust 特有的陷阱

### `todo` 是内置宏，不能做变量名
```rust
// 这样会报错：expected value, found macro `todo`
todo.update(...);  // Rust 把 todo 当成 todo!() 宏

// 解决：删掉 todo 相关代码，或换个变量名
```

### `?` 只能用在返回 Result/Option 的函数里
```rust
// run_subagent 返回 String，不能用 ?
.await?  // ← 编译错误

// 改成 match 显式处理错误
match ..send().await {
    Ok(r) => ...,
    Err(_) => break,  // 不传播，直接退出子循环
}
```

### async 递归限制
若 `run_subagent` 调用 `agent_loop`，`agent_loop` 又调用 `run_subagent`，
Rust 会报 "recursive async fn requires boxing"。
解决方案：**在 `run_subagent` 内嵌独立 for 循环，不复用 `agent_loop`**。

## 5. 今天的运行结果印证

```
[Tool: task]                    ← 父智能体触发
> task (subtask)：...           ← run_subagent 入口
[Tool: bash]                    ← 子上下文内部执行
8
共有 8 个 `.rs` 文件。          ← 子智能体摘要，返回给父
当前目录下共有 8 个 .rs 文件。  ← 父智能体最终回复
```

父 `messages` 里只有 `task` 的工具结果（一段摘要文本），
bash 的执行细节永远留在了子上下文里，随即被丢弃。

---

### 💡 第四天心得

子智能体不是炫技，是工程上对"单个 Agent 上下文会越来越脏"这个问题的务实回应。
`run_subagent` 的本质就是：开一个新对话，做完事，只把结论带回来。
后续 s15-s17 的多 Agent 协作，不过是把这个模式升级成"长期驻留的角色"而已。
