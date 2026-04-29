use dotenv::dotenv;
use std::env;
use std::io::{Read, Write};
use reqwest::Client;
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;
use std::path::PathBuf;
use std::fs::{self, OpenOptions};
use fs2::FileExt; // 引入跨平台的文件锁拓展
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// -----------------------------------------------------------------------------
// Message Types (大模型交互所需的角色定义)
// -----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
	System {
		content: String,
	},
	User {
		content: String,
	},
	Assistant {
		#[serde(skip_serializing_if = "Option::is_none")]
		content: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<Value>>,
	},
	Tool {
		content: String,
		tool_call_id: String,
	},
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InboxMessage {
  pub msg_type: String, // 消息类型，比如 "message", "broadcast"
  pub from: String, // 谁发来的
  pub content: String, // 消息正文
  pub timestamp: u64, // 发送时间戳
}

#[derive(Clone)]
pub struct MessageBus {
  pub dir: PathBuf,
}

impl MessageBus {
  pub fn new(inbox_dir: PathBuf) -> Self {
    fs::create_dir_all(&inbox_dir).unwrap_or_default();
    Self { dir: inbox_dir }
  }

  pub fn send(&self, sender: &str, to: &str, content: &str, msg_type: &str) {
    let msg = InboxMessage {
      msg_type: msg_type.to_string(),
      from: sender.to_string(),
      content: content.to_string(),
      timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
    };

    let path = self.dir.join(format!("{}.jsonl", to));
    let json_line = format!("{}\n", serde_json::to_string(&msg).unwrap());

    let mut file = OpenOptions::new().append(true).create(true).open(path).unwrap();
    file.lock_exclusive().unwrap();
    file.write_all(json_line.as_bytes()).unwrap();
    file.unlock().unwrap();
    
  }

  pub fn read_inbox(&self, name: &str) -> Vec<InboxMessage> {
    let path = self.dir.join(format!("{}.jsonl", name));
    if !path.exists() {
      return vec![];
    }

    let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.lock_exclusive().unwrap();

    let mut content = String::new();
    file.read_to_string(&mut content).unwrap();

    file.set_len(0).unwrap();
    file.unlock().unwrap();

    content.lines().filter_map(|line| {
      if line.trim().is_empty() {
        return None;
      }
      serde_json::from_str(line).ok()
    }).collect()
  }

  pub fn broadcast(&self, sender: &str, content: &str, manager: &TeammateManager) {
    // 拿到当前花名册里的所有队员名字
    let names: Vec<String> = {
      let cfg = manager.config.lock().unwrap();
      cfg.members.iter().map(|m| m.name.clone()).collect()
    };

    // 遍历所有人，除了发送者自己，全都发一份
    for to in names {
      if to != sender {
        self.send(sender, &to, content, "broadcast");
      }
    }
  }
}

// =============================================================================
// 👇👇 第三步：定义团队花名册 TeammateManager 👇👇
// =============================================================================
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Teammate {
    pub name: String,
    pub role: String,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TeamConfig {
  pub team_name: String,
  pub members: Vec<Teammate>,
}

#[derive(Clone)]
pub struct TeammateManager {
  pub dir: PathBuf,
  // 我们用 Arc<Mutex<..>> 让主线程和所有后台线程都能安全地修改 config
  pub config: Arc<Mutex<TeamConfig>>,
}

impl TeammateManager {
  pub fn new(team_dir: PathBuf) -> Self {
    // 1. 确保目录存在
    fs::create_dir_all(&team_dir).unwrap_or_default();
    let config_path = team_dir.join("config.json");

    // 2. 尝试读取文件并反序列化
    let config = if config_path.exists() {
      // 如果文件存在，用 unwrap_or_default() 兜底读取（读失败就给个空字符串）
      let text = fs::read_to_string(&config_path).unwrap_or_default();
      // 用 serde_json 解析它。如果解析失败（比如 JSON 格式坏了），同样提供一个默认值兜底
      serde_json::from_str(&text).unwrap_or(TeamConfig {
        team_name: "default".to_string(),
        members: vec![],
      })
    } else {
      // 3. 如果文件压根不存在，直接返回默认的 TeamConfig
      TeamConfig {
        team_name: "default".to_string(),
        members: vec![],
      }
    };

    // 4. 返回包好锁的大管家
    Self {
      dir: team_dir,
      config: Arc::new(Mutex::new(config)),
    }
  }

  pub fn save_config(&self) {
    let path = self.dir.join("config.json");
    // 1. 获取锁（这里不需要 mut，因为我们只是读取数据用于保存）
    let cfg = self.config.lock().unwrap();
    // 2. 把 cfg 转成漂亮排版的 json 字符串
    let json = serde_json::to_string_pretty(&*cfg).unwrap_or_default();
    // 3. 写入文件
    fs::write(path, json).unwrap_or_default();
  }

  pub fn set_status(&self, name: &str, new_status: &str) {
    // 1.这里需要可变锁，因为我们要修改里面的 members
    let mut cfg = self.config.lock().unwrap();
    // 2.遍历找人
    for member in cfg.members.iter_mut() {
      if member.name == name {
        member.status = new_status.to_string();
        break;
      }
    }
    // 3.释放锁（因为下面存盘还需要获取锁，如果在同一个函数里拿两次锁会死锁！）
    drop(cfg);

    // 4.存盘
    self.save_config();
  }

  pub fn list_members(&self) -> String {
    let cfg = self.config.lock().unwrap();
    if cfg.members.is_empty() {
      return "No teammates.".to_string();
    }
    let mut lines = vec![format!("Team: {}", cfg.team_name)];
    for m in &cfg.members {
      lines.push(format!(" {} ({}) status: {}", m.name, m.role, m.status));
    }
    lines.join("\n")
  }
}


// -----------------------------------------------------------------------------
// Base Tools (前几节课沿用的底层工具)
// -----------------------------------------------------------------------------

fn run_bash(command: &str) -> String {
    let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot"];
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
            let output = child.wait_with_output().unwrap();
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let trimmed = combined.trim().to_string();
            if trimmed.is_empty() {
                "(no output)".to_string()
            } else {
                trimmed.chars().take(50000).collect()
            }
        }
    }
}

fn safe_path(p: &str) -> Result<PathBuf, String> {
    let cwd = env::current_dir().unwrap_or_default();
    let path = cwd.join(p);
    if !path.starts_with(&cwd) {
        return Err(format!("Error: Path escapes workspace: {}", p));
    }
    Ok(path)
}

fn run_read(path_str: &str, limit: Option<usize>) -> String {
    let path = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match fs::read_to_string(path) {
        Ok(content) => {
            let mut lines: Vec<&str> = content.lines().collect();
            if let Some(l) = limit {
                if l < lines.len() {
                    let more = format!("... ({} more)", lines.len() - l);
                    lines.truncate(l);
                    let mut s = lines.join("\n");
                    s.push_str("\n");
                    s.push_str(&more);
                    return s.chars().take(50000).collect();
                }
            }
            content.chars().take(50000).collect()
        }
        Err(e) => format!("Error: {}", e),
    }
}

fn run_write(path_str: &str, content: &str) -> String {
    let path = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::write(&path, content) {
        Ok(_) => format!("Wrote {} bytes to {}", content.len(), path_str),
        Err(e) => format!("Error: {}", e),
    }
}

fn run_edit(path_str: &str, old_text: &str, new_text: &str) -> String {
    let path = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match fs::read_to_string(&path) {
        Ok(content) => {
            if !content.contains(old_text) {
                return format!("Error: Text not found in {}", path_str);
            }
            let new_content = content.replacen(old_text, new_text, 1);
            match fs::write(&path, new_content) {
                Ok(_) => format!("Edited {}", path_str),
                Err(e) => format!("Error: {}", e),
            }
        }
        Err(e) => format!("Error: {}", e),
    }
}

// =============================================================================
// 👇👇 第四步：双线 Agent 循环与通信工具 👇👇
// =============================================================================
fn team_tools() -> Value {
  json!([
    // --- 1. 底层系统能力 (Base Tools) ---
    { 
      "type": "function", 
      "function": { 
        "name": "bash", 
        "description": "Run shell command", 
        "parameters": { 
          "type": "object", 
          "properties": { 
            "command": { 
              "type": "string" 
            } 
          }, 
          "required": ["command"] 
        } 
      } 
    },
    { 
      "type": "function", 
      "function": { 
        "name": "read_file", 
        "description": "Read file", 
        "parameters": { 
          "type": "object", 
          "properties": { 
            "path": { 
              "type": "string" 
            } 
          }, 
          "required": ["path"] 
        } 
      } 
    },
    { 
      "type": "function", 
      "function": { 
        "name": "write_file", 
        "description": "Write file", 
        "parameters": { 
          "type": "object", 
          "properties": { 
            "path": { 
              "type": "string" 
            }, 
            "content": { 
              "type": "string" 
            } 
          }, 
          "required": ["path", "content"] 
        } 
      } 
    },
    { 
      "type": "function", 
      "function": { 
        "name": "edit_file", 
        "description": "Edit file contents", 
        "parameters": { 
          "type": "object", 
          "properties": { 
            "path": { 
              "type": "string" 
            }, 
            "old_text": { 
              "type": "string" 
            }, 
            "new_text": { 
              "type": "string" 
            } 
          }, 
          "required": ["path", "old_text", "new_text"] 
        } 
      } 
    },
    // --- 2. 团队协作能力 (Team Tools) ---
    {
      "type": "function",
      "function": {
          "name": "spawn_teammate",
          "description": "Spawn a persistent teammate in a background thread.",
          "parameters": {
              "type": "object",
              "properties": {
                  "name": { "type": "string" },
                  "role": { "type": "string" },
                  "prompt": { "type": "string" }
              },
              "required": ["name", "role", "prompt"]
          }
      }
    },
    {
      "type": "function",
      "function": {
          "name": "send_message",
          "description": "Send a message to another teammate's inbox.",
          "parameters": {
              "type": "object",
              "properties": {
                  "to": { "type": "string" },
                  "content": { "type": "string" }
              },
              "required": ["to", "content"]
          }
      }
    },
    {
      "type": "function",
      "function": {
          "name": "read_inbox",
          "description": "Read and clear your inbox.",
          "parameters": {
              "type": "object",
              "properties": {
                  "name": { "type": "string" }
              },
              "required": ["name"]
          }
      }
    },
    { 
      "type": "function", 
      "function": { 
        "name": "broadcast", 
        "description": "Send to all", 
        "parameters": { 
          "type": "object", 
          "properties": { 
            "content": { 
              "type": "string" 
            }
          }, 
          "required": ["content"] 
        } 
      } 
    },
    { 
      "type": "function", 
      "function": { 
        "name": "list_teammates", 
        "description": "List all teammates", 
        "parameters": { 
          "type": "object", 
          "properties": {} 
        } 
      } 
    }
  ])
}

// 后台干活的小弟的循环
async fn _teammate_loop(
  name: String,
  role: String,
  prompt: String,
  client: Client,
  api_key: String,
  base_url: String,
  model_id: String,
  bus: MessageBus,
  manager: TeammateManager,
) {
  let system = format!("You are teammate '{}', role: '{}'. You can send messages to others and use tools.", name, role);
  let mut messages: Vec<Message> = vec![Message::User { content: prompt.clone() }];
  for _ in 0..30 {
    // 1：在调用 LLM 前，先读取自己的信箱
    let inbox_msgs = bus.read_inbox(&name);
    if !inbox_msgs.is_empty() {
      let mut inbox_text = String::from("<inbox>\n");
      for m in inbox_msgs {
        inbox_text.push_str(&format!("[From {}]: {}\n", m.from, m.content));
      }
      inbox_text.push_str("</inbox>\n");
      // 把收到的信当作一次 User 输入塞给大模型
      messages.push(Message::User { content: inbox_text });
    }

    // 接下来就是标准的组装请求体
    let mut req_msgs = vec![json!(Message::System { content: system.clone() })];
    for msg in &messages { 
      req_msgs.push(json!(msg)); 
    }
    let body = json!({
      "model": model_id,
      "messages": req_msgs,
      "tools": team_tools(),
      "max_tokens": 4000
    });

    let resp = match client.post(format!("{}/v1/chat/completions", base_url))
      .header("Authorization", format!("Bearer {}", api_key))
      .header("Content-Type", "application/json")
      .json(&body)
      .send()
      .await {
        Ok(r) => r.json::<Value>().await.unwrap_or(json!({})),
        Err(_) => break, // 如果网络断了，小弟就罢工了
      };
    let choice = &resp["choices"][0];
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");
    let assistant_msg = Message::Assistant {
      content: choice["message"]["content"].as_str().map(|s| s.to_string()),
      tool_calls: choice["message"]["tool_calls"].as_array().cloned(),
    };
    messages.push(assistant_msg);

    // 如果没有叫工具，说明回答完毕，小弟的本轮思考结束
    if finish_reason != "tool_calls" {
      break;
    }
    let mut results = vec![];
    if let Some(calls) = choice["message"]["tool_calls"].as_array() {
      for tc in calls {
        let tool_name = tc["function"]["name"].as_str().unwrap_or("");
        let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
        // 这里要引入 serde_json 解析参数
        let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

        let output = match tool_name {
           // --- 基础工具 (小弟也要会！) ---
          "bash" => {
            run_bash(args["command"].as_str().unwrap_or(""))
          },
          "read_file" => {
            run_read(args["path"].as_str().unwrap_or(""), None)
          },
          "write_file" => {
            run_write(args["path"].as_str().unwrap_or(""), 
            args["content"].as_str().unwrap_or(""))
          },
          "edit_file" => {
            run_edit(args["path"].as_str().unwrap_or(""), 
            args["old_text"].as_str().unwrap_or(""), 
            args["new_text"].as_str().unwrap_or(""))
          },
          // --- 团队协作 ---
          "send_message" => {
            let to = args["to"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            bus.send(&name, to, content, "message");
            format!("Message sent to {}", to)
          },
          "read_inbox" => {
            let target = args["name"].as_str().unwrap_or("");
            let msgs = bus.read_inbox(target);
            serde_json::to_string(&msgs).unwrap_or_default()
          },
          "broadcast" => {
            let content = args["content"].as_str().unwrap_or("");
            bus.broadcast(&name, content, &manager);
            format!("Broadcast sent to team.")
          },
          "list_teammates" => {
            manager.list_members()
          },
          _ => format!("Tool not implemented here")
        };
        results.push(Message::Tool {
          tool_call_id: tc["id"].as_str().unwrap_or("").to_string(),
          content: output,
        });
      }
    }
    messages.extend(results);
  }
  manager.set_status(&name, "idle");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  dotenv().ok();
  let api_key = env::var("OPENAI_API_KEY")?;
  let base_url = env::var("OPENAI_BASE_URL").unwrap_or("https://api.openai.com".to_string());
  let model_id = env::var("OPENAI_MODEL").unwrap_or("gpt-4o".to_string());

  let client = Client::new();
  let team_dir = env::current_dir()?.join(".team");
  let bus = MessageBus::new(team_dir.join("inbox"));
  let manager = TeammateManager::new(team_dir);

  println!("Agent Teams (s09) Ready");
  let system = "You are the 'lead' agent. You can spawn teammates and send messages.";
  let mut messages: Vec<Message> = vec![];
  let mut input = String::new();

  loop {
    // 2: 老板读取信箱
    let msgs = bus.read_inbox("lead");
    for m in msgs {
      println!("\n📬 收到小弟 [{}] 的来信:\n{}\n", m.from, m.content);
    }
    print!("\ns09 >> ");
    std::io::stdout().flush()?;
    input.clear();
    std::io::stdin().read_line(&mut input)?;
    let query = input.trim();
    if query.is_empty() || query == "q" {
      break;
    }
    messages.push(Message::User {
      content: query.to_string()
    });

    for _ in 0..30 {
      // 发起请求给 LLM
      let mut req_msgs = vec![json!(Message::System {
        content: system.to_string()
      })];
      for msg in &messages {
        req_msgs.push(json!(msg));
      }
  
      // 发起请求前，加一行这个，让我们知道它没死掉
      println!("--- 正在请求网关 ({}) ---", model_id);
  
      let body = json!({
        "model": model_id,
        "messages": req_msgs,
        "tools": team_tools()
      });
  
      let resp = match client.post(format!("{}/v1/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send().await {
          Ok(r) => r.json::<Value>().await.unwrap_or(json!({})),
          Err(e) => {
            println!("Error: {}", e);
            continue;
          }
        };
      let choice = &resp["choices"][0];
      let assistant_msg = Message::Assistant {
        content: choice["message"]["content"].as_str().map(|s| s.to_string()),
        tool_calls: choice["message"]["tool_calls"].as_array().cloned(),
      };
      messages.push(assistant_msg.clone());
  
      if let Some(content) = choice["message"]["content"].as_str() {
        println!("Lead: {}", content);
      }
  
      // 处理 LLM 调用的工具
      if let Some(calls) = choice["message"]["tool_calls"].as_array() {
        let mut results = vec![];
        for tc in calls {
          let tool_name = tc["function"]["name"].as_str().unwrap_or("");
          let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
          let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
  
          let output = match tool_name {
            // --- 基础工具逻辑 ---
            "bash" => {
              run_bash(
                args["command"].as_str().unwrap_or("")
              )
            },
            "read_file" => {
              run_read(
                args["path"].as_str().unwrap_or(""), 
                None
              )
            },
            "write_file" => {
              run_write(
                args["path"].as_str().unwrap_or(""), 
                args["content"].as_str().unwrap_or("")
              )
            },
            "edit_file" => {
              run_edit(args["path"].as_str().unwrap_or(""), 
              args["old_text"].as_str().unwrap_or(""), 
              args["new_text"].as_str().unwrap_or(""))
            },
            // --- 团队协作逻辑 ---
            "spawn_teammate" => {
              // 1. 从大模型传来的 args 里提取小弟的名字、角色、提示词
              let teammate_name = args["name"].as_str().unwrap_or("").to_string();
              let teammate_role = args["role"].as_str().unwrap_or("").to_string();
              let teammate_prompt = args["prompt"].as_str().unwrap_or("").to_string();
  
              // 2. 把花名册里这个小弟的状态改为干活中
              manager.set_status(&teammate_name, "working");
  
              // 3. 克隆各种依赖，因为把闭包扔进后台线程后，它会拥有这些变量的所有权
              let c_client = client.clone();
              let c_api = api_key.clone();
              let c_base = base_url.clone();
              let c_model = model_id.clone();
              let c_bus = bus.clone();
              let c_manager = manager.clone();
  
              // 4. 发射火箭！开启后台线程独立执行小弟死循环
              tokio::spawn(async move {
                _teammate_loop(
                  teammate_name,
                  teammate_role,
                  teammate_prompt,
                  c_client,
                  c_api,
                  c_base,
                  c_model,
                  c_bus,
                  c_manager
                ).await;
              });
  
              // 5. 告诉老板的 LLM：小弟已经成功启程
              format!("Spawned teammate [{}].", args["name"].as_str().unwrap_or(""))
            },
            "send_message" => {
              let to = args["to"].as_str().unwrap_or("");
              let content = args["content"].as_str().unwrap_or("");
              bus.send("lead", to, content, "message");
              format!("Message sent to {}", to)
            },
            "read_inbox" => {
              let msgs = bus.read_inbox("lead");
              serde_json::to_string(&msgs).unwrap_or_default()
            },
            "broadcast" => {
              let content = args["content"].as_str().unwrap_or("");
              bus.broadcast("lead", content, &manager);
              format!("Broadcast sent to all teammates.")
            },
            "list_teammates" => {
              manager.list_members()
            },
            _ => format!("Tool not implemented")
          };
  
          results.push(Message::Tool {
            tool_call_id: tc["id"].as_str().unwrap_or("").to_string(),
            content: output,
          });
        }
        messages.extend(results);
      } else {
        break;
      }
    }
  }
  Ok(())
}
