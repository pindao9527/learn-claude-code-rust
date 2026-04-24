use dotenv::dotenv;
use std::env;
use std::io::{self, Write};
use reqwest::Client;
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;
use std::path::PathBuf;
use std::collections::HashMap;
use std::fs;
use thiserror::Error;
use walkdir::WalkDir;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

// 记录单个后台任务的状态
pub struct BgTask {
  pub status: String,
  pub command: String,
  pub result: Option<String>,
}

// 记录完成后的通知
pub struct Notification {
  pub task_id: String,
  pub status: String,
  pub result: String,
}

// 背景任务管理器。注意我们对需要共享变更的属性包上了 Arc<Mutex<...>>
#[derive(Clone)]
pub struct BackgroundManager {
  // 存储任务详情，键为任务 ID
  pub tasks: Arc<Mutex<HashMap<String, BgTask>>>,
  // 存储执行完毕的通知，以便大模型读取
  pub notifications: Arc<Mutex<Vec<Notification>>>,
  // 一个极其简单的自增 ID 计数器，用以生成 task_id
  pub next_id: Arc<Mutex<u32>>,
}

impl BackgroundManager {
  // 构造函数
  pub fn new() -> Self {
    Self {
      tasks: Arc::new(Mutex::new(HashMap::new())),
      notifications: Arc::new(Mutex::new(Vec::new())),
      next_id: Arc::new(Mutex::new(1)),
    }
  }

  // 发射后台命令，它会立即返回一个 task_id
  pub fn run(&self, command: String) -> String {
    // 1. 生成唯一标识
    let mut id_lock = self.next_id.lock().unwrap();
    let task_id = format!("bg_{}", *id_lock);
    *id_lock += 1;
    // 手动丢弃锁，避免后续操作持有太久
    drop(id_lock);

    // 2. 登记状态：标记为 running
    self.tasks.lock().unwrap().insert(
      task_id.clone(),
      BgTask {
        status: "running".to_string(),
        command: command.clone(),
        result: None,
      }
    );

    // 3. 克隆 manager，这只会增加 Arc 的引用计数，代价极小
    let manager_clone = self.clone();
    let task_id_clone = task_id.clone();
    let cmd_clone = command.clone();

    // 4. 将阻塞的命令调用丢给 Tokio 的专设阻塞线程池
    // move 关键字意味着闭包拿走了 manager_clone 和 cmd_clone 的所有权！
    tokio::task::spawn_blocking(move || {
      // 调用原本在 s07 里的那个阻塞版 run_bash
      let output = run_bash(&cmd_clone);

      // 跑完后，加锁更新状态
      if let Ok(mut tasks) = manager_clone.tasks.lock() {
        if let Some(t) = tasks.get_mut(&task_id_clone) {
          t.status = "completed".to_string();
          t.result = Some(output.clone());
        }
      }

      // 塞入通知队列
      if let Ok(mut notifs) = manager_clone.notifications.lock() {
        notifs.push(Notification {
          task_id: task_id_clone,
          status: "completed".to_string(),
          result: output,
        });
      }

    });

    format!("Background task {} started", task_id)
  }

  // 瞬间抽空所有积压的通知
  pub fn drain_notifications(&self) -> Vec<Notification> {
    let mut notifs = self.notifications.lock().unwrap();
    // std::mem::take 是极好用的 Rust 魔法：抽走 Vec 中的数据，并在原地留下一个空 Vec！
    std::mem::take(&mut *notifs)
  }

  // 检查某一个/所有后台任务状态（供大模型调用）
  pub fn check(&self, task_id: Option<String>) -> String {
    let tasks = self.tasks.lock().unwrap();
    if let Some(id) = task_id {
      if let Some(t) = tasks.get(&id) {
        let res = t.result.as_deref().unwrap_or("(running)");
        format!("[{}] {}\n{}", t.status, t.command, res)
      } else {
        format!("Error: Unknown task {}", id)
      }
    } else {
      let mut lines = Vec::new();
      for (id, t) in tasks.iter() {
        lines.push(format!("{}: [{}] {}", id, t.status, t.command));
      }
      if lines.is_empty() {
        "No background tasks.".to_string()
      } else {
        lines.join("\n")
      }
    }
  }
}

// 这里由于 async fn in trait 仍在演进或者考虑到使用泛型对象 Box<dyn Compressor>，
// 最好返回一个使用 Box 包裹的 Future，这是目前兼容性最好的面向接口异步编程写法（如果不用第三方库）。
pub trait Compressor {
  // 我们的压缩可能是“就地修改旧数据”，也可能要返回结果，所以传入 &mut Vec<Message> 可变借用
  // 返回一个被 Pin 住的基于堆分配的 Future（其实这里我们可以体验纯手写，不引入第三方 async-trait 宏）
  fn compress<'a>(
    &'a self,
    messages: &'a mut Vec<Message>,
    client: &'a Client,
    model_id: &'a str
  ) -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>>;
}

// 我们定义一个结构体，把要保留的近期工具条数变成可配置的
pub struct MicroCompressor {
  pub keep_recent: usize,
}

impl Compressor for MicroCompressor {
  fn compress<'a>(
    &'a self,
    messages: &'a mut Vec<Message>,
    _client: &'a Client, // 微压缩不需要网络请求，用 _ 忽略
    _model_id: &'a str
  ) -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>> {
    Box::pin(async move {
      let mut tool_count = 0;

      // 巧用 Rust 的双端迭代器，从后往前遍历消息（rev() 将迭代器反转），
      // 这样最先遇到的 Tool 消息一定是最新的！
      for msg in messages.iter_mut().rev() {
        // 如果恰好匹配出 Tool 消息，解构出可变借用的 content
        if let Message::Tool {content, ..} = msg {
          tool_count += 1;
          // 如果发现已经跳过了最新执行的 N 次工具调用
          if tool_count > self.keep_recent {
             // 且内容较长，才实行切除术
             if content.len() > 120 {
               // 就地点石成金：直接替换这块内存存储的字符串！
               *content = "[Earlier tool result compacted. Re-run the tool if you need full detail.]".to_string();
             }
          }
        }
      }
      // 返回 false 表示：我只做了一点修剪剪裁，并不意味着当前上下文已经被“完全重置”
      // 允许其他可能的压缩策略继续工作
      Ok(false)
    })
  }
}

// 我们把配置与请求鉴权独立保存在这个压缩器的状态里！
pub struct SummaryCompressor {
  pub max_len: usize,
  pub api_key: String,
  pub base_url: String,
}

impl Compressor for SummaryCompressor {
  fn compress<'a>(
    &'a self,
    messages: &'a mut Vec<Message>,
    client: &'a Client,
    model_id: &'a str
  ) -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>> {
    Box::pin(async move {
      // 直接通过 JSON 序列化估算文字长度
      let serialized = serde_json::to_string(messages).unwrap_or_default();

      if serialized.len() < self.max_len {
        return Ok(false); // 还不到火候，直接放行
      }

      println!("\n\x1B[35m[Auto Compact触发] 上下文过长({} 字符)，正在呼叫 LLM 浓缩精华...\x1B[0m", serialized.len());

      // 截短一部分防止超长
      let cut_len = std::cmp::min(80000, serialized.len());
      let prompt = format!("Summarize this coding-agent conversation so work can continue.\n\
                 Preserve:\n\
                 1. The current goal\n\
                 2. Important findings and decisions\n\
                 3. Files read or changed\n\
                 4. Remaining work\n\
                 Be compact but concrete.\n\n{}", &serialized[..cut_len]);
      let body = serde_json::json!({
        "model": model_id,
        "messages": [{"role":"user", "content": prompt}],
        "max_tokens": 2000
      });

      // 以一部全新的网络请求专门获取精华总结
      let resp = client
        .post(format!("{}/v1/chat/completions", self.base_url))
        .header("Authorization", format!("Bearer {}", self.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

      let summary = resp["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("No summary generated.")
        .to_string();

      // 重点：暴力美学清空原有所有数组！！！
      messages.clear();

      // 塞入这粒浓缩的知识胶囊作为真正的“最初始”对话
      messages.push(Message::User {
        content: format!(
          "This conversation was compacted so the agent can continue working.\n\n{}",
          summary
        )
      });
      Ok(true) // 返回 true 告诉后续，这里的记忆已经被“重写”了
    })
  }
}


#[derive(Error, Debug)]
pub enum SkillError {
  // 1.无缝包装系统 IO 错误， 当底层发生 IO 错误时，通过 ? 操作符，自动将 io::Error 转换成 SkillError::Io
  #[error("文件读取失败：{0}")]
  Io(#[from] std::io::Error),

  // 2.无缝包装 walkdir 遍历错误
  #[error("目录遍历异常：{0}")]
  WalkDir(#[from] walkdir::Error),

  // 3.业务逻辑错误， 比如解析失败
  #[error("技能配置解析失败， 找不到对应路径或格式错误：{0}")]
  ParseError(String),

  // 4.用户/大模型调用了不存在的特殊技能
  #[error("未找到对应名称的技能：{0}")]
  NotFound(String),
}

#[derive(Debug, Clone)]
pub struct SkillManifest {
  pub name: String,
  pub description: String,
  pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SkillDocument {
  pub manifest: SkillManifest,
  pub body: String,
}

pub struct SkillRegistry{
  pub skills_dir: PathBuf,
  pub documents: HashMap<String, SkillDocument>,
}

impl SkillRegistry {
  // 1.初始化的时候直接顺手加载完所有数据
  pub fn new(skills_dir: PathBuf) -> Self {
    let mut registry = Self {
      skills_dir,
      documents: HashMap::new(),
    };

    // 外部入口我们不抛错了，打印一下就行
    if let Err(e) = registry.load_all() {
      eprintln!("警告： 技能库加载遇到错误： {}", e);
    }
    registry
  }

  // 2.核心加载逻辑：大量使用 Result 和 ? 进行优雅失败
  fn load_all(&mut self) -> Result<(), SkillError> {
    if !self.skills_dir.exists() {
      return Ok(());
    }

    // 递归遍历
    for entry in WalkDir::new(&self.skills_dir) {
      let entry = entry?; // 将 Walkdir 的底层 Error 转成 SkillError
      let path = entry.path();

      // 只要文件名叫 SKILL.md 就是我们要找的技能文件
      if path.is_file() && path.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
        let content = fs::read_to_string(path)?;
        // 将 std::io::Error 转换过去
        let (meta, body) = Self::parse_frontmatter(&content);

        let name = meta.get("name").map(|s| s.to_string()).unwrap_or_else(|| path.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned());

        let description = meta.get("description").map(|s| s.to_string()).unwrap_or_else(|| "No description".to_string());

        let manifest = SkillManifest {
          name: name.clone(),
          description,
          path: path.to_path_buf(),
        };

        self.documents.insert(
          name,
          SkillDocument { manifest, body: body.to_string() },
        );
      }
    }
    Ok(())
  }

  // 3.(辅助方法) 肉眼解剖文本，剥离顶部的 --- key:value ---
  fn parse_frontmatter(text: &str) -> (HashMap<String, String>, String) {
    let mut meta = HashMap::new();
    let mut body = text.to_string();

    if text.starts_with("---\n") {
      // 找到截断点：寻找下一个 '---' (从第4个字符开始搜，避开开头的)
      if let Some(end_idx) = text[4..].find("---\n") {
        let actual_end_idx = 4 + end_idx;
        let frontmatter = &text[4..actual_end_idx];

        for line in frontmatter.lines() {
          // Rust string.split_once('字符') 极好用
          if let Some((k, v)) = line.split_once(":") {
            meta.insert(k.trim().to_string(), v.trim().to_string());
          }
        }
        body = text[actual_end_idx + 4..].trim().to_string();
      }
    }
    (meta, body)
  }

  // 4.提供给 prompt 的小抄，由于只是读取没有改写所以就不可能抛错啦
  pub fn describe_available(&self) -> String {
    if self.documents.is_empty() {
      return "(no skills available)".to_string();
    }
    let mut lines = Vec::new();
    for (name, doc) in &self.documents {
      lines.push(format!("- {}: {}", name, doc.manifest.description));
    }
    lines.sort();
    lines.join("\n")
  }

  // 5.大模型挂载特定技能时的取书方法（如果没这本书，抛出 NotFound 业务错误！）
  pub fn load_full_text(&self, name: &str) -> Result<String, SkillError> {
    let doc = self.documents.get(name).ok_or_else(|| {
      let mut known: Vec<&String> = self.documents.keys().collect();
      known.sort();
      let known_list = if known.is_empty() { 
        "(none)".to_string()
      } else {
        known.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("，")
      };
      // 这里我们手动触发了一个自定义业务错误！
      SkillError::NotFound(format!("Unknown skill '{}'.Available skills: {}", name, known_list))
    })?;

    Ok(format!("<skill name=\"{}\">\n{}\n</skill>", doc.manifest.name, doc.body))
  }
}

const SYSTEM: &str = "ou are a coding agent. Use the task tool to delegate exploration or subtasks.";
const SUBAGENT_SYSTEM: &str = "You are a coding subagent. Complete the given task, then summarize your findings.";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Task {
  pub id: u32,
  pub subject: String,
  pub description: String,
  pub status: String,
  pub blocked_by: Vec<u32>,
  pub owner: String,
}

pub struct TaskManager {
  pub dir: PathBuf,
  next_id: u32,
}

impl TaskManager {
  pub fn new(dir: PathBuf) -> Self {
    std::fs::create_dir_all(&dir).ok();
    let next_id = Self::max_id(&dir) + 1;
    Self { dir, next_id }
  }

  fn max_id(dir: &PathBuf) -> u32 {
    // 遍历 dir 下所有 task_N.json，提取最大的 N
    let mut max = 0u32;
    if let Ok(entries) = std::fs::read_dir(dir) {
      for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        // s 长这样："task_3.json"
        // 你来填：从 s 中提取数字，更新 max
        if let Some(n) = s.strip_prefix("task_")
        .and_then(|s| s.strip_suffix(".json"))
        .and_then(|s| s.parse::<u32>().ok()) {
          if n > max { max = n; }
        }
      }
    }
    max
  }

  fn load(&self, task_id: u32) -> Result<Task, String> {
    let path = self.dir.join(format!("task_{}.json", task_id));
    let content = std::fs::read_to_string(&path)
      .map_err(|_| format!("Task {} not found", task_id))?;
    serde_json::from_str(&content)
      .map_err(|e| format!("Parse error: {}", e))
  }

  fn save(&self, task: &Task) -> Result<(), String> {
    let path = self.dir.join(format!("task_{}.json", task.id));
    let json = serde_json::to_string_pretty(task)
      .map_err(|e| format!("Serialize error: {}", e))?;
    std::fs::write(&path,json)
      .map_err(|e| format!("Write error: {}", e))
  }

  pub fn create(&mut self, subject: String, description: String) -> String {
    let task = Task {
      id: self.next_id,
      subject,
      description,
      status: "pending".to_string(),
      blocked_by: vec![],
      owner: String::new(),
    };
    // 1. 调用 self.save(&task)，忽略错误用 .ok()
    // 2. self.next_id += 1
    // 3. 把 task 序列化成 JSON 字符串返回
    //    用 serde_json::to_string_pretty(&task).unwrap_or_default()
    self.save(&task).ok();
    self.next_id += 1;
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  pub fn list_all(&self) -> String {
    let mut tasks: Vec<Task> = vec![];
    if let Ok(entries) = std::fs::read_dir(&self.dir) {
      let mut ids: Vec<u32> = entries
        .flatten()
        .filter_map(|e| {
          let name = e.file_name();
          let s = name.to_string_lossy();
          s.strip_prefix("task_")
            .and_then(|s| s.strip_suffix(".json"))
            .and_then(|s| s.parse::<u32>().ok())
        })
        .collect();
      ids.sort();
      for id in ids {
        if let Ok(task) = self.load(id) {
          tasks.push(task);
        }
      }
    }
    if tasks.is_empty() {
      return "No tasks.".to_string();
    }
    // 把 tasks 格式化成字符串
    // 参考 Python：
    // [ ] #1: subject (blocked by: [2])
    // [>] #2: subject
    // [x] #3: subject
    tasks.iter().map(|t| {
      let marker = match t.status.as_str() { 
        "pending" => "[ ]",
        "in_progress" => "[>]",
        "completed" => "[x]",
        _ => "[?]"
      };
      let blocked = if t.blocked_by.is_empty() {
        String::new()
      } else {
        format!(" (blocked by: {:?})", t.blocked_by)
      };
      format!("{} #{}: {}{}", marker, t.id, t.subject, blocked)
    }).collect::<Vec<_>>().join("\n")
  }

  pub fn get(&self, task_id: u32) -> String {
    match self.load(task_id) {
      Ok(task) => serde_json::to_string_pretty(&task)
      .unwrap_or_default(),
      Err(e) => format!("Error: {}", e),
    }
  }
  

  pub fn update(&mut self, task_id: u32, status: Option<String>) -> String {
    let mut task = match self.load(task_id) {
      Ok(t) => t,
      Err(e) => return format!("Error: {}", e),
    };
    if let Some(s) = status {
      // 验证 status 合法
      if !["pending", "in_progress", "completed"].contains(&s.as_str()) {
        return format!("Error: Invalid status: {}", s);
      }
      task.status = s.clone();
      if s == "completed" {
        self.clear_dependency(task_id);
      }
    }
    self.save(&task).ok();
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  fn clear_dependency(&self, completed_id: u32) {
     // 遍历所有任务，从 blocked_by 中移除 completed_id
    // 提示：用 list_all 里类似的遍历方式，load 每个任务，修改后 save 回去
    if let Ok(entries) = std::fs::read_dir(&self.dir) {
      let ids: Vec<u32> = entries
        .flatten()
        .filter_map(|e| {
          let name = e.file_name();
          let s = name.to_string_lossy();
          s.strip_prefix("task_")
            .and_then(|s| s.strip_suffix(".json"))
            .and_then(|s| s.parse::<u32>().ok())
        })
        .collect();
      for id in ids {
        if let Ok(mut task) = self.load(id) {
          if task.blocked_by.contains(&completed_id) {
            task.blocked_by.retain(|&x| x != completed_id);
            self.save(&task).ok();
          }
        }
      }
    }
  }
}

// #[serde(tag = "role")] 是 Rust 枚举的序列化/反序列化宏
// 它告诉 serde：序列化时，在 JSON 对象里加一个 key 为 "role" 的字段，
// 并把枚举成员名（System/User/Assistant/Tool）作为这个字段的值。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
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

fn child_tools() -> Value {
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
    // --- load_skill ---
    {
      "type":"function",
      "function": {
        "name": "load_skill",
        "description": "Load the full body of a named skill into the current context.",
        "parameters": {
          "type": "object",
          "properties": {
            "name": {
              "type": "string"
            }
          },
          "required": ["name"]
        }
      }
    },
  ])
}

async fn run_subagent(
  prompt: &str,
  client: &Client,
  api_key: &str,
  base_url: &str,
  model_id: &str,
  registry: &SkillRegistry,
) -> String {
    // 1. 全新上下文（不继承父对话）
    let mut sub_messages: Vec<Message> = vec![
      Message:: User{ content: prompt.to_string() }
    ];

    // 2. 最多循环 30 次
    for _ in 0..30 {
      // 3.组装 request_messages (System + sub_messages)
      // 注意：用 SUBAGENT_SYSTEM, 不是 SYSTEM
      let mut request_messages = vec![json!(Message::System{ content: SUBAGENT_SYSTEM.to_string() })];

      for msg in sub_messages.iter() {
        request_messages.push(json!(msg));
      }

      let body = json!({
        "model": model_id,
        "messages": request_messages,
        "tools": child_tools(),
        "max_tokens": 8000
      });

      // 4.调用 LLM （和 agent_loop 里一样的HTTP请求）
      // 注意：工具用 child_tools(), 不是 parent_tools()
      let resp = match client.post(format!("{}/v1/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await {
          Ok(r) => match r.json::<Value>().await {
            Ok(v) => v,
            Err(_) => break,
          }
          Err(_) => break,
        };

      // 5.把 assistant 响应 push 进 sub_messages
      let choice = &resp["choices"][0];
      let finish_reason = choice["finish_reason"].as_str().unwrap_or("");
      let assistant_msg = Message::Assistant { content: 
        choice["message"]["content"].as_str().map(|s| s.to_string()),
        tool_calls: choice["message"]["tool_calls"].as_array().cloned(),
      };
      sub_messages.push(assistant_msg);

      // 6.如果 finish_reason != "tool_call", break
      if finish_reason != "tool_calls" {
        break;
      }

      // 7.遍历 tool_calls， 执行工具 (bash/read/write/edit)
      // 结果 push 进 sub_messages (作为 Tool 消息)
      let mut results: Vec<Message> = vec![]; // 存储Message
      if let Some(tool_calls) = choice["message"]["tool_calls"].as_array() {
        for tc in tool_calls {
          let tool_name = tc["function"]["name"].as_str().unwrap_or(""); // 获取工具名称
          let fn_args = tc["function"]["arguments"].as_str().unwrap_or("{}"); // 获取工具参数
          let args: Value = serde_json::from_str(fn_args).unwrap_or(json!({})); // 解析工具参数

          // 打印工具名称
          println!("\x1B[33m[Tool: {}]\x1B[0m", tool_name);

          // 核心工具执行逻辑
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
            },
            "load_skill" => {
              let name = args["name"].as_str().unwrap_or("");
              // 因为我们在第5步巧妙地封好了 Result 结构，这里处理极度干净
              match registry.load_full_text(name) {
                Ok(text) => text,
                Err(e) => format!("Error: {}", e) // 直接打印我们的 Domain Error!
              }
            },
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
      sub_messages.extend(results);
    }

    // 8. 只返回最后一条  Assistant 消息的文本
    // sub_messages 的其余内容全部丢弃
    if let Some(Message::Assistant{ content: Some(text), .. }) = sub_messages.last() {
      text.clone()
    } else {
      "(no summary)".to_string()
    }
}

fn parent_tools() -> Value {
  // child_tools 的 4 个工具 + task
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
    // --- task ---
    {
      "type": "function",
      "function": {
        "name": "task",
        "description": "Spawn a subagent with fresh context. It shares the filesystem but not conversation history.",
        "parameters": {
          "type": "object",
          "properties": {
            "prompt": { "type": "string" },
            "description": { "type": "string" }
          },
          "required": ["prompt"]
        }
      }
    },
    // --- load_skill ---
    {
      "type":"function",
      "function": {
        "name": "load_skill",
        "description": "Load the full body of a named skill into the current context.",
        "parameters": {
          "type": "object",
          "properties": {
            "name": {
              "type": "string"
            }
          },
          "required": ["name"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "task_create",
        "description": "Create a new task.",
        "parameters": {
          "type": "object",
          "properties": {
            "subject": {"type": "string"},
            "description": {"type": "string"}
          },
          "required": ["subject"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "task_list",
        "description": "List all tasks with status.",
        "parameters": {"type": "object", "properties": {}}
      }
    },
    {
      "type": "function",
      "function": {
        "name": "task_get",
        "description": "Get full details of a task by ID.",
        "parameters": {
          "type": "object",
          "properties": {"task_id": {"type": "integer"}},
          "required": ["task_id"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "task_update",
        "description": "Update a task status.",
        "parameters": {
          "type": "object",
          "properties": {
              "task_id": {"type": "integer"},
              "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
          },
          "required": ["task_id"]
        }
      }
    },
    // --- background_run ---
    {
      "type": "function",
      "function": {
        "name": "background_run",
        "description": "Run command in background thread. Returns task_id immediately.",
        "parameters": {
          "type": "object",
          "properties": { "command": { "type": "string" } },
          "required": ["command"]
        }
      }
    },
    // --- check_background ---
    {
      "type": "function",
      "function": {
        "name": "check_background",
        "description": "Check background task status. Omit task_id to list all.",
        "parameters": {
          "type": "object",
          "properties": { "task_id": { "type": "string" } }
        }
      }
    },
  ])
}

async fn agent_loop(
  client: &Client,
  api_key: &str,
  base_url: &str,
  model_id: &str,
  system: &str,
  messages: &mut Vec<Message>, // &mut 表示可变引用，允许在循环中修改消息列表
  registry: &SkillRegistry,
  compressors: &[Box<dyn Compressor>], // 接收类型擦除后的 Trait Object 数组
  tasks: &mut TaskManager,
  bg_manager: &BackgroundManager,
) -> Result<(), Box<dyn std::error::Error>> {
  loop {
    // 进入循环第一件事就是排空队列
    let notifs = bg_manager.drain_notifications();
    if !notifs.is_empty() && !messages.is_empty() {
      let mut notif_text = String::new();
      for n in notifs {
        notif_text.push_str(&format!("[bg:{}] {}: {}\n", n.task_id, n.status, n.result));
      }
      messages.push(Message::User {
        content: format!("<background-results>\n{}\n</background-results>", notif_text)
      });
    }

    // 1.在每次即将发起模型请求前，挨个过一遍压缩网关
    for comp in compressors {
      // 利用动态分发调用实现好的 compress
      let fully_compacted = comp.compress(messages, client, model_id).await?;
      if fully_compacted {
        // 如果 SummaryCompressor 返回了 true，代表历史全清空了，无需后面的压缩器再跑
        break;
      }
    }


    // 2. 准备消息列表
    let mut request_messages = vec![json!(Message::System{ content: system.to_string() })];
    
    // 3. 这里的重点！我们通过 extend(messages) 来加入历史
    // 因为 Message 实现了 Serialize，json! 会自动处理它
    for msg in messages.iter() {
      request_messages.push(json!(msg));
    }
    

    let body = json!({
      "model": model_id,
      "messages": request_messages,
      "tools": parent_tools(),
      "max_tokens": 8000
    });

    // 4. 发送请求（OpenAI 协议）
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .json::<Value>()
        .await?;

    // 5. 解析 OpenAI 响应格式
    let choice = &resp["choices"][0];
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");
    let assistant_msg = Message::Assistant {
      content: choice["message"]["content"].as_str().map(|s| s.to_string()),
      tool_calls: choice["message"]["tool_calls"].as_array().cloned(),
    };

    // 6.注意：这里直接 push 我们刚构造的强类型枚举
    messages.push(assistant_msg);

    // 7. 检查 finish_reason：不是 tool_calls 就跳出循环
    if finish_reason != "tool_calls" {
      return Ok(());
    }

    // 8. 遍历 tool_calls，执行命令
    let mut results: Vec<Message> = vec![]; // 存储Message
    if let Some(tool_calls) = choice["message"]["tool_calls"].as_array() {
      for tc in tool_calls {
        let tool_name = tc["function"]["name"].as_str().unwrap_or(""); // 获取工具名称
        let fn_args = tc["function"]["arguments"].as_str().unwrap_or("{}"); // 获取工具参数
        let args: Value = serde_json::from_str(fn_args).unwrap_or(json!({})); // 解析工具参数

        // 打印工具名称
        println!("\x1B[33m[Tool: {}]\x1B[0m", tool_name);

        // 核心工具执行逻辑
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
          },
          "task" => {
            let desc = args["description"].as_str().unwrap_or("subtask");
            let prompt = args["prompt"].as_str().unwrap_or("");
            let safe_prompt: String = prompt.chars().take(80).collect();
            println!("\x1B[35m> task ({})：{}\x1B[0m", desc, safe_prompt);
            run_subagent(prompt, client, api_key, base_url, model_id, registry).await
          },
          "load_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            match registry.load_full_text(name) {
              Ok(text) => text,
              Err(e) => format!("Error: {}", e)
            }
          },
          "task_create" => {
            let subject = args["subject"].as_str().unwrap_or("").to_string();
            let desc = args["description"].as_str().unwrap_or("").to_string();
            tasks.create(subject, desc)
          },
          "task_list" => tasks.list_all(),
          "task_get" => {
            let id = args["task_id"].as_u64().unwrap_or(0) as u32;
            tasks.get(id)
          },
          "task_update" => {
            let id = args["task_id"].as_u64().unwrap_or(0) as u32;
            let status = args["status"].as_str().map(|s| s.to_string());
            tasks.update(id, status)
          },
          "background_run" => {
            let command = args["command"].as_str().unwrap_or("").to_string();
            bg_manager.run(command) // 直接抛给我们的管家！
          },
          "check_background" => {
            let task_id = args["task_id"].as_str().map(|s| s.to_string());
            bg_manager.check(task_id)
          },
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

    // 9. 把工具结果逐条追加到历史，然后回到 loop 顶部
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
  let skills_dir = env::current_dir()?.join("skills");
  let registry = SkillRegistry::new(skills_dir);
  let tasks_dir = env::current_dir()?.join(".tasks");
  let mut tasks = TaskManager::new(tasks_dir);
  let bg_manager = BackgroundManager::new();

  // 将静态 prompt 与动态读取到的“小抄（可用技能清单）”进行拼装
  let system = format!("{}\n\nUse load_skill when a task needs 
  specialized instructions before 
  you act.\nSkills available:\n{}", 
  SYSTEM, registry.describe_available());

  println!("\x1B[36mRust s08 >> 已就绪! (使用模型：{})\x1B[0m", model_id);

  // 启动 REPL 主循环前，实例化我们的压缩策略数组：
  // 这里必须用 Box 裹住不同的类型，否则 Vec 不能允许放大小不一的结构体
  let mut compressors: Vec<Box<dyn Compressor>> = Vec::new();

  // 1. 微压缩器，只留最后 3 条工具调用
  compressors.push(Box::new(MicroCompressor { keep_recent: 3}));
  // 2. 总结压缩器，这里为了快速测试，我把阈值设为比较小的 1000 字符（正常用应设为几万）
  compressors.push(Box::new(SummaryCompressor {
    max_len: 80000,
    api_key: api_key.clone(),
    base_url: base_url.clone(),
  }));

  // 然后再进入 loop REPL 循环。
  let mut history: Vec<Message> = vec![];

  loop {
    // 1. 打印提示符
    print!("\x1B[36ms08 >> \x1B[0m");
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
    if let Err(e) = agent_loop(&client, &api_key, &base_url, &model_id, &system, &mut history, &registry, &compressors, &mut tasks, &bg_manager).await {
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