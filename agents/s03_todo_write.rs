use dotenv::dotenv;
use std::env;
use std::io::{self, Write};
use reqwest::Client;
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

#[derive(Debug, Clone)]
struct PlanItem {
  content: String,
  status: String,
  active_form: String,
}

struct TodoManager {
  items: Vec<PlanItem>,
  rounds_since_update: u32,
}

impl TodoManager {
  fn new() -> Self {
    TodoManager {
      items: Vec::new(),
      rounds_since_update:0,
    }
  }
  fn render(&self) -> String {
    if self.items.is_empty() {
      return "No session play yet.".to_string();
    }

    let mut lines: Vec<String> = self.items.iter().map(|item| {
      let marker = match item.status.as_str() {
        "in_progress" => "[>]",
        "completed"   => "[x]",
        _             => "[ ]",
      };
      let mut line = format!("{} {}", marker, item.content);
      if item.status == "in_progress" && !item.active_form.is_empty() {
        line.push_str(&format!(" ({})", item.active_form));
      }
      line
    }).collect();

    let completed = self.items.iter().filter(|i| i.status == "completed").count();
    lines.push(format!("\n({}/{} completed)", completed, self.items.len()));


    lines.join("\n")
  }
  fn update(&mut self, items: &Value) -> String {
    let arr = match items.as_array() {
      Some(a) => a,
      None => return "Error: items must be an array".to_string(),
    };

    if arr.len() > 12 {
      return "Error: Keep the session plan short (max 12 items)".to_string();
    }

    let in_progress = arr.iter()
        .filter(|i| i["status"].as_str().unwrap_or("pending") == "in_progress")
        .count();
    
    if in_progress > 1 {
      return "Error: Only one plan item can be in_progress".to_string();
    }

    self.items = arr.iter().map(|i| PlanItem {
      content: i["content"].as_str().unwrap_or("").to_string(),
      status: i["status"].as_str().unwrap_or("pending").to_string(),
      active_form: i["activeForm"].as_str().unwrap_or("").to_string(),
    }).collect();

    self.rounds_since_update = 0;
    self.render()
  }

  fn note_round(&mut self) {
    self.rounds_since_update += 1;
  }

  fn reminder(&self) -> Option<String> {
    if self.items.is_empty() || self.rounds_since_update < 3 {
      return None;
    }
    Some("<reminder>Refresh your current plan before continuing.</reminder>".to_string())
  }
}

// #[serde(tag = "role")] 是 Rust 枚举的序列化/反序列化宏
// 它告诉 serde：序列化时，在 JSON 对象里加一个 key 为 "role" 的字段，
// 并把枚举成员名（System/User/Assistant/Tool）作为这个字段的值。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "role", rename_all = "lowercase")]
enum Message {
  System { content: String },
  User { content: String },
  Assistant {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<Value>>,
  },
  // 工具调用（助手调用工具后，模型返回的）
  Tool {
    content: String,
    tool_call_id: String,
  }
}

fn run_bash(command: &str) -> String {
  let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", ">/dev/"];
  if dangerous.iter().any(|d| command.contains(d)) {
    return "Error: Dangerous command blocked".to_string();
  }

  let mut child = match std::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
  {
    Ok(c) => c,
    Err(e) => return format!("Error: {}", e),
  };

  let timeout = std::time::Duration::from_secs(120);
  match child.wait_timeout(timeout).unwrap_or(None) {
    None => {
      let _ = child.kill();
      "Error: Timeout (120s)".to_string()
    }
    Some(_) => {
      let output = child.wait_with_output().unwrap_or_else(|e| panic!("{}", e));
      let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
      );
      let trimmed = combined.trim().to_string();
      if trimmed.is_empty() { "(no output)".to_string() } else { trimmed.chars().take(50000).collect() }
    }
  }
}

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

fn run_write(path_str: &str, content: &str) -> String {
  let path = match safe_path(path_str) {
    Ok(p) => p,
    Err(e) => return e,
  };

  if let Some(parent) = path.parent() {
    if let Err(e) = std::fs::create_dir_all(parent) {
      return format!("Error creating directory: {}", e);
    }
  }

  match std::fs::write(&path, content) {
    Ok(_) => format!("Successfully wrote to {}", path_str),
    Err(e) => format!("Error writing file: {}", e),
  }
}

fn run_edit(path_str: &str, old_text: &str, new_text: &str) -> String {
  let path = match safe_path(path_str) {
    Ok(p) => p,
    Err(e) => return e,
  };
  match std::fs::read_to_string(&path) {
    Ok(content) => {
      // 安全检查：确保旧文本确实存在，防止误删
      if !content.contains(old_text) {
        return format!("Error: Could not find exact text match in {}", path_str);
      }

      // 替换文本。Rust 的 replace 会替换所有匹配项
      // 实际工程中，我们通常只替换第一个匹配项，或者使用 diff 算法。
      // 如果需要只替换第一个匹配项，可以使用 replace_range
      let new_content = content.replace(old_text, new_text);
      match std::fs::write(&path, new_content){
        Ok(_) => format!("Successfully edited {}", path_str),
        Err(e) => format!("Error writing file: {}", e),
      }
    }

    Err(e) => format!("Error reading file: {}", e),
  }
}

fn get_tools_config() -> Value {
  json!([
    // ---- bash ----
    {
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
    },
    // --- read_file ---
    {
      "type": "function",
      "function": {
        "name": "read_file",
        "description": "Read contents of a file.",
        "parameters": {
          "type": "object",
          "properties": { "path": { "type": "string"}},
          "required": [ "path" ]
        }
      }
    },
    // --- write_file ---
    {
      "type": "function",
      "function": {
        "name": "write_file",
        "description": "Write content to a file.",
        "parameters": {
          "type": "object",
          "properties": {
            "path": { "type": "string"},
            "content": { "type": "string"}
          },
          "required": ["path", "content"]
        }
      }
    },
    // --- edit_file ---
    {
      "type": "function",
      "function": {
        "name": "edit_file",
        "description": "Replace exact text in a file.",
        "parameters": {
          "type": "object",
          "properties": {
            "path": { "type": "string" },
            "old_text": { "type": "string" },
            "new_text": { "type": "string" }
          },
          "required": ["path", "old_text", "new_text"]
        }
      }
    },
    // --- todo ---
    {
      "type": "function",
      "function":{
        "name": "todo",
        "description": "Rewrite the current session plan for multi-step work.",
        "parameters": {
          "type": "object",
          "properties": {
            "items":{
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "content": { "type": "string"},
                  "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"]
                  },
                  "activeForm": {
                    "type": "string"
                  }
                },
                "required": ["content", "status"]
              }
            }
          },
          "required": ["items"]
        }
      }
    }
  ])
}

async fn agent_loop(
  client: &Client,
  api_key: &str,
  base_url: &str,
  model_id: &str,
  system: &str,
  messages: &mut Vec<Message>, // &mut 表示可变引用，允许在循环中修改消息列表
  todo: &mut TodoManager,
) -> Result<(), Box<dyn std::error::Error>> {
  loop {
    // 1. 准备消息列表
    let mut request_messages = vec![json!(Message::System{ content: system.to_string() })];
    
    // 2. 这里的重点！我们通过 extend(messages) 来加入历史
    // 因为 Message 实现了 Serialize，json! 会自动处理它
    for msg in messages.iter() {
      request_messages.push(json!(msg));
    }
    

    let body = json!({
      "model": model_id,
      "messages": request_messages,
      "tools": get_tools_config(),
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
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");
    let assistant_msg = Message::Assistant {
      content: choice["message"]["content"].as_str().map(|s| s.to_string()),
      tool_calls: choice["message"]["tool_calls"].as_array().cloned(),
    };

    // 4.注意：这里直接 push 我们刚构造的强类型枚举
    messages.push(assistant_msg);

    // 5. 检查 finish_reason：不是 tool_calls 就跳出循环
    if finish_reason != "tool_calls" {
      return Ok(());
    }

    // 6. 遍历 tool_calls，执行命令
    let mut results: Vec<Message> = vec![]; // 存储Message
    if let Some(tool_calls) = choice["message"]["tool_calls"].as_array() {
      for tc in tool_calls {
        let tool_name = tc["function"]["name"].as_str().unwrap_or(""); // 获取工具名称
        let fn_args = tc["function"]["arguments"].as_str().unwrap_or("{}"); // 获取工具参数
        let args: Value = serde_json::from_str(fn_args).unwrap_or(json!({})); // 解析工具参数

        // 打印工具名称
        println!("\x1B[33m[Tool: {}]\x1B[0m", tool_name);

        // 
        let output = match tool_name {
          "bash" => {
            let command = args["command"].as_str().unwrap_or("");
            run_bash(command)
          },
          "read_file" => {
            let path = args["path"].as_str().unwrap_or("");
            run_read(path) // 调用读文件函数
          },
          "write_file" => {
            let path = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            run_write(path, content)
          },
          "edit_file" => {
            let path = args["path"].as_str().unwrap_or("");
            let old_text = args["old_text"].as_str().unwrap_or("");
            let new_text = args["new_text"].as_str().unwrap_or("");
            run_edit(path, old_text, new_text)
          }
          "todo" => {
            todo.update(&args["items"])
          }
          _ => format!("Unknow tool: {}", tool_name),
        };

        println!("{}", &output[..output.len().min(200)]);
        
        // push tool result
        results.push(Message::Tool {
          tool_call_id:
          tc["id"].as_str().unwrap_or("").to_string(),
          content: output,
        });
      }
    }

    // 7.todo nag 逻辑
    let used_todo = choice["message"]["tool_calls"]
        .as_array()
        .map(|tcs| tcs.iter().any(|tc| tc["function"]["name"] == "todo"))
        .unwrap_or(false);

    if used_todo {
      todo.rounds_since_update = 0;
    } else {
      todo.note_round();
      if let Some(reminder) = todo.reminder() {
        results.insert(0, Message::Tool {
          tool_call_id: "reminder".to_string(),
          content: reminder,
        })
      }
    }

    // 8. 把工具结果逐条追加到历史，然后回到 loop 顶部
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

  println!("\x1B[36mRust s03 >> 已就绪! (使用模型：{})\x1B[0m", model_id);

  // REPL 主循环
  let mut history: Vec<Message> = vec![];

  let mut todo = TodoManager::new();

  loop {
    // 1. 打印提示符
    print!("\x1B[36ms03 >> \x1B[0m");
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
    history.push(Message::User { content: query.to_string() });

    // 5. 调用 agent_loop 并处理错误
    if let Err(e) = agent_loop(&client, &api_key, &base_url, &model_id, &system, &mut history, &mut todo).await {
      eprintln!("Error: {}", e);
    }

    // 6. 打印助手的回复（OpenAI 格式：message.content 是字符串）
    if let Some(Message::Assistant { content: Some(text), ..}) = history.last() {
      println!("{}", text);
    }
    println!();
  }

  Ok(())
}