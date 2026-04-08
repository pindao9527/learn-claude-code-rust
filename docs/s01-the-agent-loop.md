# s01: The Agent Loop (Agent 循环)

`[ s01 ] s02 > s03 > s04 > s05 > s06 | s07 > s08 > s09 > s10 > s11 > s12`

> *"One loop & Bash is all you need"* -- 一个工具 + 一个循环 = 一个 Agent。
>
> **Harness 层**: 循环 -- 模型与真实世界的第一道连接。

## 问题

语言模型能推理代码, 但碰不到真实世界 -- 不能读文件、跑测试、看报错。没有循环, 每次工具调用你都得手动把结果粘回去。你自己就是那个循环。

## 解决方案

```
+--------+      +-------+      +---------+
|  User  | ---> |  LLM  | ---> |  Tool   |
| prompt |      |       |      | execute |
+--------+      +---+---+      +----+----+
                    ^                |
                    |   tool_result  |
                    +----------------+
                    (loop until stop_reason != "tool_use")
```

一个退出条件控制整个流程。循环持续运行, 直到模型不再调用工具。

## 工作原理

1. 用户 prompt 作为第一条消息。

```rust
history.push(json!({ "role": "user", "content": query}));
```

1. 将消息和工具定义一起发给 LLM。

```rust
let resp = client
    .post(format!("{}/v1/chat/completions", base_url))
    .json(&json!({
        "model": model_id,
        "messages": full_messages,
        "tools": TOOLS,
        "max_tokens": 8000
    }))
    .send()
    .await?
    .json::<Value>()
    .await?;
```

1. 追加助手响应。检查 `finish_reason` -- 如果模型没有调用工具, 结束。

```rust
let message = &resp["choices"][0]["message"];
let finish_reason = resp["choices"][0]["finish_reason"].as_str().unwrap_or("");
messages.push(message.clone());

if finish_reason != "tool_calls" {
    return Ok(());
}
```

1. 执行每个工具调用, 收集结果, 逐条追加到历史。回到第 2 步。

```rust
let mut results: Vec<Value> = vec![];
if let Some(tool_calls) = message["tool_calls"].as_array() {
    for tc in tool_calls {
        let args: Value = serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}")).unwrap();
        let command = args["command"].as_str().unwrap_or("");
        let output = run_bash(command);
        results.push(json!({
            "role": "tool",
            "tool_call_id": tc["id"],
            "content": output
        }));
    }
}
messages.extend(results);
```

组装为一个完整函数:

```rust
async fn agent_loop(
  client: &Client,
  messages: &mut Vec<Value>,
) -> Result<(), Box<dyn std::error::Error>> {
  loop {
    let resp = client.post(URL).json(&body).send().await?.json::<Value>().await?;
    let message = &resp["choices"][0]["message"];
    let finish_reason = resp["choices"][0]["finish_reason"].as_str().unwrap_or("");
    messages.push(message.clone());

    if finish_reason != "tool_calls" { return Ok(()); }

    let mut results: Vec<Value> = vec![];
    if let Some(tool_calls) = message["tool_calls"].as_array() {
      for tc in tool_calls {
        let args: Value = serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))?;
        let output = run_bash(args["command"].as_str().unwrap_or(""));
        results.push(json!({"role": "tool", "tool_call_id": tc["id"], "content": output}));
      }
    }
    messages.extend(results);
  }
}
```

不到 30 行, 这就是整个 Agent。后面 11 个章节都在这个循环上叠加机制 -- 循环本身始终不变。

## 变更内容

| 组件          | 之前       | 之后                           |
|---------------|------------|--------------------------------|
| Agent loop    | (无)       | `loop` + finish_reason         |
| Tools         | (无)       | `bash` (单一工具)              |
| Messages      | (无)       | 累积式消息列表 (`Vec<Value>`)  |
| Control flow  | (无)       | `finish_reason != "tool_calls"`|

## 试一试

```sh
# cd learn-claude-code-rust
cargo run --bin s01
```

试试这些 prompt (英文 prompt 对 LLM 效果更好, 也可以用中文):

1. `Create a file called hello.py that prints "Hello, World!"`
2. `List all Python files in this directory`
3. `What is the current git branch?`
4. `Create a directory called test_output and write 3 files in it`
