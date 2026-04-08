use dotenv::dotenv;
use std::env;
use std::io::{self, Write};
use reqwest::Client;
use serde_json::{json, Value};

fn run_bash(command: &str) -> String {
  // 危险命令拦截
  let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", ">/dev/"];
  if dangerous.iter().any(|d| command.contains(d)) {
    return "Error: Dangerous command blocked".to_string();
  }

  // 用标准库执行 shell 命令
  let result = std::process::Command::new("sh")
      .arg("-c")
      .arg(command)
      .output(); // 同步执行，等待结果
  
  match result {
    Ok(output) => {
      let stdout = String::from_utf8_lossy(&output.stdout);
      let stderr = String::from_utf8_lossy(&output.stderr);
      let combined = format!("{}{}", stdout, stderr);
      let trimmed = combined.trim().to_string();
      if trimmed.is_empty() {
        "(no output)".to_string()
      } else {
        trimmed.chars().take(50000).collect() // 截断太长的输出
      }
    }
    Err(e) => format!("Error: {}", e),
  }
  
}

async fn agent_loop(
  client: &Client,
  api_key: &str,
  base_url: &str,
  model_id: &str,
  system: &str,
  messages: &mut Vec<Value>, // &mut 表示可变引用，允许在循环中修改消息列表
) -> Result<(), Box<dyn std::error::Error>> {
  loop {
    // OpenAI 协议：system 放在 messages 数组最前面
    let mut full_messages = vec![json!({"role": "system", "content": system})];
    full_messages.extend(messages.clone());

    let body = json!({
      "model": model_id,
      "messages": full_messages,
      "tools": [{
        "type": "function",
        "function": {
          "name": "bash",
          "description": "Run a shell command.",
          "parameters": {
            "type": "object",
            "properties": { "command": {
              "type": "string"
            }},
            "required": ["command"]
          }
        }
      }],
      "max_tokens": 8000
    });

    // 2. 发送请求（OpenAI 协议）
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .json::<Value>()
        .await?;

    // 3. 解析 OpenAI 响应格式
    let choice = &resp["choices"][0];
    let message = &choice["message"];
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

    // 4. 把助手的回复追加到消息历史
    messages.push(message.clone());

    // 5. 检查 finish_reason：不是 tool_calls 就跳出循环
    if finish_reason != "tool_calls" {
      return Ok(());
    }

    // 6. 遍历 tool_calls，执行命令
    let mut results: Vec<Value> = vec![];
    if let Some(tool_calls) = message["tool_calls"].as_array() {
      for tc in tool_calls {
        let fn_args = tc["function"]["arguments"].as_str().unwrap_or("{}");
        let args: Value = serde_json::from_str(fn_args).unwrap_or(json!({}));
        let command = args["command"].as_str().unwrap_or("");
        println!("\x1B[33m$ {}\x1B[0m", command);
        let output = run_bash(command);
        println!("{}", &output[..output.len().min(200)]);
        results.push(json!({
          "role": "tool",
          "tool_call_id": tc["id"],
          "content": output
        }));
      }
    }

    // 7. 把工具结果逐条追加到历史，然后回到 loop 顶部
    messages.extend(results);
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  dotenv().ok(); // 1.加载.env文件

  // 2.环境变量（使用 OpenAI 兼容配置）
  let api_key = env::var("OPENAI_API_KEY")?;
  let base_url = env::var("OPENAI_BASE_URL").unwrap_or("https://api.openai.com".to_string());
  let model_id = env::var("OPENAI_MODEL").unwrap_or("gpt-4o".to_string());

  // 3.初始化客户端
  let client = Client::new();
  let system = format!("You are a coding agent. Use bash to solve tasks. Act, don't explain.");

  println!("\x1B[36mRust s01 >> 已就绪! (使用模型：{})\x1B[0m", model_id);

  // REPL 主循环
  let mut history: Vec<Value> = vec![];
  loop {
    // 1. 打印提示符
    print!("\x1B[36ms01 >> \x1B[0m");
    io::stdout().flush()?; // io::Write 的 flush() 方法用于刷新缓冲区，确保提示符立即显示
    
    // 2. 读取用户输入
    let mut query = String::new();
    io::stdin().read_line(&mut query)?;
    let query = query.trim();

    // 3. 退出
    if query.is_empty() || query == "q" || query == "exit" {
      break;
    }

    // 4. 把用户输入追加到消息历史
    history.push(json!({ "role": "user", "content": query}));

    // 5. 调用 agent_loop 并处理错误
    if let Err(e) = agent_loop(&client, &api_key, &base_url, &model_id, &system, &mut history).await {
      eprintln!("Error: {}", e);
    }

    // 6. 打印助手的回复（OpenAI 格式：message.content 是字符串）
    if let Some(last) = history.last() {
      if let Some(text) = last["content"].as_str() {
        println!("{}", text);
      }
    }
    println!();
  }

  Ok(())
}