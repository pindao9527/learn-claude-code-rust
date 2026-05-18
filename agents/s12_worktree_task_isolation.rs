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
use std::collections::HashMap;
use tokio::time::{sleep, Duration};

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
  #[serde(default)]
  pub extra: Option<serde_json::Value>,
}

// =============================================================================
// 新增：状态机 enum + 请求追踪器
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum RequestStatus {
  Pending,
  Approved,
  Rejected,
}

pub struct ShutdownTracker {
  pub target: String,
  pub status: RequestStatus,
}

pub struct PlanTracker {
  pub from: String,
  pub plan: String,
  pub status: RequestStatus,
}

const POLL_INTERVAL: u64 = 5; // 秒
const IDLE_TIMEOUT: u64 = 60; // 秒

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
      extra: None,
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

  pub fn send_with_extra(
    &self,
    sender: &str,
    to: &str,
    content: &str,
    msg_type: &str,
    extra: Option<serde_json::Value>,
  ){
    let msg = InboxMessage {
      msg_type: msg_type.to_string(),
      from: sender.to_string(),
      content: content.to_string(),
      timestamp: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs(),
      extra,
    };

    let path = self.dir.join(format!("{}.jsonl", to));
    let json_line = format!("{}\n", serde_json::to_string(&msg).unwrap());
    let mut file = OpenOptions::new()
      .append(true)
      .create(true)
      .open(path)
      .unwrap();
    file.lock_exclusive().unwrap();
    file.write_all(json_line.as_bytes()).unwrap();
    file.unlock().unwrap();
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

fn now_secs() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// =============================================================================
// EventBus：追加式生命周期事件日志
// =============================================================================

pub struct EventBus {
  pub path: PathBuf,
}

impl EventBus {
  pub fn new(path: PathBuf) -> Self {
    // 确保父目录存在，文件不存在时创建空文件
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).unwrap_or_default();
    }
    if !path.exists() {
      fs::write(&path, "").unwrap_or_default();
    }
    Self { path }
  }

  pub fn emit(
    &self,
    event: &str,
    task: Option<Value>,
    worktree: Option<Value>,
    error: Option<&str>,
  ) {
    let mut payload = json!({
      "event": event,
      "ts": now_secs(),
      "task": task.unwrap_or(json!({})),
      "worktree": worktree.unwrap_or(json!({})),
    });
    if let Some(e) = error {
      payload["error"] = json!(e);
    }
    // append(true)：只往末尾加，不覆盖
    if let Ok(mut file) = OpenOptions::new().append(true).create(true).open(&self.path) {
      let _ = writeln!(file, "{}", payload);
    }
  }

  pub fn list_recent(&self, limit: usize) -> String {
    let n = limit.max(1).min(200);
    let content = fs::read_to_string(&self.path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    // 取最后 n 行
    let recent = if lines.len() > n {
      &lines[lines.len() - n..]
    } else {
      &lines[..]
    };
    let items: Vec<Value> = recent.iter().map(|line| {
      serde_json::from_str(line).unwrap_or(json!({
        "event": "parse_error", "raw": line
      }))
    }).collect();
    serde_json::to_string_pretty(&items).unwrap_or_default()
  }
}

impl Clone for EventBus {
  fn clone(&self) -> Self {
    Self {
      path: self.path.clone()
    }
  }
}

// =============================================================================
// TaskManager：任务板（.tasks/task_N.json）
// =============================================================================
pub struct TaskManager {
  pub dir: PathBuf,
}

impl TaskManager {
  pub fn new(dir: PathBuf) -> Self {
    fs::create_dir_all(&dir).unwrap_or_default();
    Self { dir }
  }

  // 扫描目录，找当前最大 ID
  fn max_id(&self) -> u64 {
    fs::read_dir(&self.dir).ok()
      .into_iter().flatten()
      .filter_map(|e| e.ok())
      .filter_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("task_") && name.ends_with(".json") {
          name[5..name.len() - 5].parse::<u64>().ok()
        } else {
          None
        }
      })
      .max()
      .unwrap_or(0)
  }

  fn path(&self, id: u64) -> PathBuf {
    self.dir.join(format!("task_{}.json", id))
  }

  fn load(&self, id: u64) -> Result<Value, String> {
    let p = self.path(id);
    if !p.exists() {
      return Err(format!("Task {} not found", id));
    }
    let text = fs::read_to_string(p).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
  }

  fn save(&self, task: &Value) {
    let id = task["id"].as_u64().unwrap_or(0);
    let content = serde_json::to_string_pretty(task).unwrap_or_default();
    fs::write(self.path(id), content).unwrap_or_default();
  }

  pub fn exists(&self, id: u64) -> bool {
    self.path(id).exists()
  }

  pub fn create(&self, subject: &str, description: &str) -> String {
    let id = self.max_id() + 1;
    let task = json!({
      "id": id,
      "subject": subject,
      "description": description,
      "status": "pending",
      "owner": null,
      "worktree": "",
      "blockedBy": [],
      "created_at": now_secs(),
      "updated_at": now_secs(),
    });
    self.save(&task);
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  pub fn get(&self, id: u64) -> String {
    match self.load(id) {
      Ok(t) => serde_json::to_string_pretty(&t).unwrap_or_default(),
      Err(e) => format!("Error: {}", e),
    }
  }

  pub fn update(&self, id: u64, status: Option<&str>, owner: Option<&str>) -> String {
    let mut task = match self.load(id) {
      Ok(t) => t,
      Err(e) => return format!("Error: {}", e),
    };
    if let Some(s) = status {
      task["status"] = json!(s);
    }
    if let Some(o) = owner {
      task["owner"] = json!(o);
    }
    task["updated_at"] = json!(now_secs());
    self.save(&task);
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  // 绑定 worktree：同时把 pending → in_progress
  pub fn bind_worktree(&self, id: u64, worktree: &str) -> String {
    let mut task = match self.load(id) {
      Ok(t) => t,
      Err(e) => return format!("Error: {}", e),
    };
    task["worktree"] = json!(worktree);
    if task["status"].as_str() == Some("pending") {
      task["status"] = json!("in_progress");
    }
    task["updated_at"] = json!(now_secs());
    self.save(&task);
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  pub fn unbind_worktree(&self, id: u64) -> String {
    let mut task = match self.load(id) {
      Ok(t) => t,
      Err(e) => return format!("Error: {}", e),
    };
    task["worktree"] = json!("");
    task["updated_at"] = json!(now_secs());
    self.save(&task);
    serde_json::to_string_pretty(&task).unwrap_or_default()
  }

  pub fn list_all(&self) -> String {
    let mut tasks: Vec<Value> = 
    fs::read_dir(&self.dir).ok()
    .into_iter().flatten()
    .filter_map(|e| e.ok())
    .filter(|e| e.file_name().to_string_lossy()
    .starts_with("task_"))
    .filter_map(|e| fs::read_to_string(e.path()).ok())
    .filter_map(|s| serde_json::from_str(&s).ok())
    .collect();
    if tasks.is_empty() {
      return "No tasks.".to_string();
    }
    tasks.sort_by_key(|t| t["id"].as_u64().unwrap_or(0));
    tasks.iter().map(|t| {
      let marker = match t["status"].as_str() {
        Some("pending") => "[ ]",
        Some("in_progress") => "[>]",
        Some("completed") => "[x]",
        _ => "[?]",
      };
      let owner = t["owner"].as_str().filter(|s| !s.is_empty())
      .map(|o| format!(" owner={}", o)).unwrap_or_default();
      let wt = t["worktree"].as_str().filter(|s| !s.is_empty())
      .map(|w| format!(" wt={}", w)).unwrap_or_default();
      format!("{} #{}: {}{}{}", marker, t["id"], t["subject"].as_str().unwrap_or(""), owner, wt)
    }).collect::<Vec<_>>().join("\n")
  }
}

impl Clone for TaskManager {
  fn clone(&self) -> Self { 
    Self { 
      dir: self.dir.clone() 
    }
  }
}

// =============================================================================
// WorktreeManager：管理 git worktree（.worktrees/）
// =============================================================================

pub struct WorktreeManager {
    pub repo_root: PathBuf,
    pub dir: PathBuf,
    pub index_path: PathBuf,
    pub tasks: TaskManager,
    pub events: EventBus,
    pub git_available: bool,
}

impl WorktreeManager {
    pub fn new(repo_root: PathBuf, tasks: TaskManager, events: EventBus) -> Self {
        let dir = repo_root.join(".worktrees");
        fs::create_dir_all(&dir).unwrap_or_default();
        let index_path = dir.join("index.json");
        if !index_path.exists() {
            fs::write(&index_path, "{\"worktrees\":[]}").unwrap_or_default();
        }
        // 检测当前目录是否在 git repo 内
        let git_available = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&repo_root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        Self { dir, index_path, tasks, events, git_available, repo_root }
    }

    // 核心：调用 git 子进程，在 repo_root 目录下执行
    fn run_git(&self, args: &[&str]) -> Result<String, String> {
        if !self.git_available {
            return Err("Not in a git repo. worktree tools require git.".to_string());
        }
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.repo_root)   // 必须在 repo 根目录执行
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let msg = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            return Err(msg.trim().to_string());
        }
        let s = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(if s.trim().is_empty() { "(no output)".to_string() } else { s.trim().to_string() })
    }

    fn load_index(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(&self.index_path).unwrap_or_default())
            .unwrap_or(json!({"worktrees": []}))
    }

    fn save_index(&self, data: &Value) {
        fs::write(&self.index_path, serde_json::to_string_pretty(data).unwrap_or_default())
            .unwrap_or_default();
    }

    fn find(&self, name: &str) -> Option<Value> {
        self.load_index()["worktrees"]
            .as_array()?
            .iter()
            .find(|wt| wt["name"].as_str() == Some(name))
            .cloned()
    }

    // 校验 worktree 名称：只允许字母数字 . _ -，1-40字符
    fn validate_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() || name.len() > 40 {
            return Err("Name must be 1-40 chars".to_string());
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-') {
            return Err("Name: only letters, numbers, . _ - allowed".to_string());
        }
        Ok(())
    }

    pub fn create(&self, name: &str, task_id: Option<u64>, base_ref: &str) -> String {
        if let Err(e) = self.validate_name(name) {
            return format!("Error: {}", e);
        }
        if self.find(name).is_some() {
            return format!("Error: Worktree '{}' already exists", name);
        }
        if let Some(id) = task_id {
            if !self.tasks.exists(id) {
                return format!("Error: Task {} not found", id);
            }
        }
        let path = self.dir.join(name);
        let branch = format!("wt/{}", name);
        self.events.emit(
            "worktree.create.before",
            task_id.map(|id| json!({"id": id})),
            Some(json!({"name": name, "base_ref": base_ref})),
            None,
        );
        match self.run_git(&["worktree", "add", "-b", &branch,
                             &path.to_string_lossy(), base_ref]) {
            Err(e) => {
                self.events.emit("worktree.create.failed",
                    task_id.map(|id| json!({"id": id})),
                    Some(json!({"name": name})), Some(&e));
                format!("Error: {}", e)
            }
            Ok(_) => {
                let entry = json!({
                    "name": name,
                    "path": path.to_string_lossy(),
                    "branch": branch,
                    "task_id": task_id,
                    "status": "active",
                    "created_at": now_secs(),
                });
                let mut idx = self.load_index();
                idx["worktrees"].as_array_mut()
                    .unwrap().push(entry.clone());
                self.save_index(&idx);
                if let Some(id) = task_id {
                    self.tasks.bind_worktree(id, name);
                }
                self.events.emit("worktree.create.after",
                    task_id.map(|id| json!({"id": id})),
                    Some(json!({"name": name, "branch": branch, "status": "active"})),
                    None);
                serde_json::to_string_pretty(&entry).unwrap_or_default()
            }
        }
    }

    pub fn list_all(&self) -> String {
        let idx = self.load_index();
        let wts = idx["worktrees"].as_array().cloned().unwrap_or_default();
        if wts.is_empty() { return "No worktrees in index.".to_string(); }
        wts.iter().map(|wt| {
            let suffix = wt["task_id"].as_u64()
                .map(|id| format!(" task={}", id)).unwrap_or_default();
            format!("[{}] {} -> {} ({}){}",
                wt["status"].as_str().unwrap_or("unknown"),
                wt["name"].as_str().unwrap_or(""),
                wt["path"].as_str().unwrap_or(""),
                wt["branch"].as_str().unwrap_or("-"),
                suffix)
        }).collect::<Vec<_>>().join("\n")
    }

    pub fn status(&self, name: &str) -> String {
        let wt = match self.find(name) {
            Some(w) => w,
            None => return format!("Error: Unknown worktree '{}'", name),
        };
        let path = PathBuf::from(wt["path"].as_str().unwrap_or(""));
        if !path.exists() { return format!("Error: Path missing: {:?}", path); }
        let out = std::process::Command::new("git")
            .args(["status", "--short", "--branch"])
            .current_dir(&path)
            .output();
        match out {
            Ok(o) => {
                let s = format!("{}{}", String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr));
                if s.trim().is_empty() { "Clean worktree".to_string() } else { s.trim().to_string() }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    pub fn run_in(&self, name: &str, command: &str) -> String {
        let dangerous = ["rm -rf /", "sudo", "shutdown", "> /dev/"];
        if dangerous.iter().any(|d| command.contains(d)) {
            return "Error: Dangerous command blocked".to_string();
        }
        let wt = match self.find(name) {
            Some(w) => w,
            None => return format!("Error: Unknown worktree '{}'", name),
        };
        let path = PathBuf::from(wt["path"].as_str().unwrap_or(""));
        if !path.exists() { return format!("Error: Path missing: {:?}", path); }
        match std::process::Command::new("sh")
            .arg("-c").arg(command)
            .current_dir(&path)
            .output()
        {
            Ok(out) => {
                let s = format!("{}{}", String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr));
                if s.trim().is_empty() { "(no output)".to_string() } else { s.trim().to_string() }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    pub fn keep(&self, name: &str) -> String {
        let wt = match self.find(name) {
            Some(w) => w,
            None => return format!("Error: Unknown worktree '{}'", name),
        };
        let mut idx = self.load_index();
        let mut kept = None;
        for item in idx["worktrees"].as_array_mut().unwrap_or(&mut vec![]) {
            if item["name"].as_str() == Some(name) {
                item["status"] = json!("kept");
                item["kept_at"] = json!(now_secs());
                kept = Some(item.clone());
            }
        }
        self.save_index(&idx);
        self.events.emit("worktree.keep",
            wt["task_id"].as_u64().map(|id| json!({"id": id})),
            Some(json!({"name": name, "status": "kept"})), None);
        kept.map(|k| serde_json::to_string_pretty(&k).unwrap_or_default())
            .unwrap_or(format!("Error: Unknown worktree '{}'", name))
    }

    pub fn remove(&self, name: &str, force: bool, complete_task: bool) -> String {
        let wt = match self.find(name) {
            Some(w) => w,
            None => return format!("Error: Unknown worktree '{}'", name),
        };
        let task_id = wt["task_id"].as_u64();
        self.events.emit("worktree.remove.before",
            task_id.map(|id| json!({"id": id})),
            Some(json!({"name": name, "path": wt["path"]})), None);
        let mut args = vec!["worktree", "remove"];
        if force { args.push("--force"); }
        let path_str = wt["path"].as_str().unwrap_or("");
        args.push(path_str);
        if let Err(e) = self.run_git(&args) {
            self.events.emit("worktree.remove.failed",
                task_id.map(|id| json!({"id": id})),
                Some(json!({"name": name})), Some(&e));
            return format!("Error: {}", e);
        }
        if complete_task {
            if let Some(id) = task_id {
                let subject = self.tasks.get(id);
                self.tasks.update(id, Some("completed"), None);
                self.tasks.unbind_worktree(id);
                self.events.emit("task.completed",
                    Some(json!({"id": id, "subject": subject, "status": "completed"})),
                    Some(json!({"name": name})), None);
            }
        }
        let mut idx = self.load_index();
        for item in idx["worktrees"].as_array_mut().unwrap_or(&mut vec![]) {
            if item["name"].as_str() == Some(name) {
                item["status"] = json!("removed");
                item["removed_at"] = json!(now_secs());
            }
        }
        self.save_index(&idx);
        self.events.emit("worktree.remove.after",
            task_id.map(|id| json!({"id": id})),
            Some(json!({"name": name, "status": "removed"})), None);
        format!("Removed worktree '{}'", name)
    }
}

impl Clone for WorktreeManager {
    fn clone(&self) -> Self {
        Self {
            repo_root: self.repo_root.clone(),
            dir: self.dir.clone(),
            index_path: self.index_path.clone(),
            tasks: self.tasks.clone(),
            events: self.events.clone(),
            git_available: self.git_available,
        }
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

fn handle_shutdown_request(
  teammate: &str,
  bus: &MessageBus,
  trackers: &Arc<Mutex<HashMap<String, ShutdownTracker>>>,
) -> String {
  // 生成唯一 ID（用时间戳简化，真实项目用 uuid）
  let req_id = format!(
    "{:x}",
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_millis()
  )[..8]
    .to_string();

  // 写入追踪器
  {
    let mut map = trackers.lock().unwrap();
    map.insert(
      req_id.clone(),
      ShutdownTracker {
        target: teammate.to_string(),
        status: RequestStatus::Pending,
      },
    );
  }

  // 发送消息给 Teammate
  bus.send_with_extra(
    "lead",
    teammate,
    "Please confirm shutdown.",
    "shutdown_request",
    Some(serde_json::json!({ "request_id": req_id })),
  );

  format!("Shutdown request sent to {} (req_id: {})", teammate, req_id)
}

fn handle_plan_review(
  req_id: &str,
  approve: bool,
  feedback: &str,
  to: &str,
  bus: &MessageBus,
  plan_trackers: &Arc<Mutex<HashMap<String, PlanTracker>>>,
) -> String {
  // 更新追踪器状态
  {
    let mut map = plan_trackers.lock().unwrap();
    if let Some(tracker) = map.get_mut(req_id) {
      tracker.status = if approve {
        RequestStatus::Approved
      } else {
        RequestStatus::Rejected
      };
    }
  }

  // 回复 Teammate
  bus.send_with_extra(
    "lead",
    to,
    feedback,
    "plan_approval_response",
    Some(serde_json::json!({
      "request_id": req_id,
      "approve": approve,
    })),
  );

  format!(
    "Plan {} (req_id: {}) → {}",
    to,
    req_id,
    if approve { "APPROVED" } else { "REJECTED" }
  )
}

fn make_identity_block(name: &str, role: &str, team_name: &str) -> Message {
  Message::User {
    content: format!(
      "<identity>You are '{}', role: {}, team: {}. Continue your work.</identity>",
      name, role, team_name
    )
  }
}

fn scan_unclaimed_tasks(tasks_dir: &PathBuf) -> Vec<Value> {
  fs::create_dir_all(tasks_dir).unwrap_or_default();
  let mut unclaimed = vec![];
  if let Ok(entries) = fs::read_dir(tasks_dir) {
    let mut paths: Vec<_> = entries
      .filter_map(|e| e.ok())
      .filter(|e| e.file_name().to_string_lossy().starts_with("task_"))
      .collect();
    paths.sort_by_key(|e| e.file_name());
    for entry in paths {
      if let Ok(text) = fs::read_to_string(entry.path()) {
        if let Ok(task) = serde_json::from_str::<Value>(&text) {
          let is_pending = task["status"].as_str() == Some("pending");
          let no_owner = task["owner"].is_null();
          let not_blocked = task["blockedBy"].as_array()
            .map_or(true, |a| a.is_empty());
          if is_pending && no_owner && not_blocked {
            unclaimed.push(task);
          }
        }
      }
    }
  }
  unclaimed
}

fn claim_task(
  tasks_dir: &PathBuf,
  task_id: u64,
  owner: &str,
  claim_lock: &Arc<Mutex<()>>,
) -> String {
  let _guard = claim_lock.lock().unwrap();
  let path = tasks_dir.join(format!("task_{}.json", task_id));
  if !path.exists() {
    return format!("Error: Task {} not found", task_id);
  }
  let text = match fs::read_to_string(&path) {
    Ok(t) => t,
    Err(e) => return format!("Error: {}", e),
  };
  let mut task: Value = match serde_json::from_str(&text) {
    Ok(v) => v,
    Err(e) => return format!("Error: {}", e),
  };
  if !task["owner"].is_null() {
    return format!("Error: Task {} already claimed by {}", task_id, task["owner"]);
  }
  if task["blockedBy"].as_array().map_or(false, |a|!a.is_empty()) {
    return format!("Error: Task {} is blocked", task_id);
  }
  task["owner"] = json!(owner);
  task["status"] = json!("in_progress");
  match fs::write(&path, serde_json::to_string_pretty(&task).unwrap_or_default()) {
    Ok(_) => format!("Claimed task #{} for {}", task_id, owner),
    Err(e) => format!("Error: {}", e),
  }
}

// =============================================================================
// 👇👇 第四步：双线 Agent 循环与通信工具 👇👇
// =============================================================================
fn worktree_tools() -> Value {
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
    },
    {
      "type": "function",
      "function": {
        "name": "shutdown_request",
        "description": "Send a graceful shutdown request to a teammate.",
        "parameters": {
          "type": "object",
          "properties": {
            "teammate": { "type": "string" }
          },
          "required": ["teammate"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "shutdown_response",
        "description": "Respond to a shutdown request (teammate use only).",
        "parameters": {
          "type": "object",
          "properties": {
            "request_id": { "type": "string" },
            "approve": { "type": "boolean" }
          },
          "required": ["request_id", "approve"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "plan_approval",
        "description": "Submit a plan for Lead approval (teammate) or approve/reject a plan (lead).",
        "parameters": {
          "type": "object",
          "properties": {
            "request_id": { "type": "string" },
            "plan": { "type": "string" },
            "approve": { "type": "boolean" },
            "feedback": { "type": "string" }
          },
          "required": []
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "idle",
        "description": "Signal that you have no more work. Enters idle polling phase.",
        "parameters": {
          "type": "object",
          "properties": {}
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "claim_task",
        "description": "Claim an unclaimed task from the .tasks/ board by ID.",
        "parameters": {
          "type": "object",
          "properties": {
            "task_id": { "type": "integer" }
          },
          "required": ["task_id"]
        }
      }
    },
    // --- 3. 任务管理工具 ---
    { 
      "type": "function", "function": {
        "name": "task_create",
        "description": "Create a new task on the task board.",
        "parameters": { 
          "type": "object",
          "properties": {
            "subject": { "type": "string" },
            "description": { "type": "string" }
          }, 
          "required": ["subject"]
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "task_list",
        "description": "List all tasks with status, owner, and worktree binding.",
        "parameters": { 
          "type": "object", "properties": {} 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "task_get",
        "description": "Get task details by ID.",
        "parameters": { 
          "type": "object",
          "properties": { 
            "task_id": { 
              "type": "integer" 
            } 
          },
          "required": ["task_id"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "task_update",
        "description": "Update task status or owner.",
        "parameters": { 
          "type": "object",
          "properties": {
              "task_id": { "type": "integer" },
              "status": { "type": "string" },
              "owner": { "type": "string" }
          }, 
          "required": ["task_id"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "task_bind_worktree",
        "description": "Bind a task to a worktree name.",
        "parameters": { 
          "type": "object",
          "properties": {
              "task_id": { "type": "integer" },
              "worktree": { "type": "string" }
          }, 
          "required": ["task_id", "worktree"] 
        }
      }
    },
    // --- 4. Worktree 工具 ---
    { 
      "type": "function", "function": {
        "name": "worktree_create",
        "description": "Create a git worktree and optionally bind it to a task.",
        "parameters": { 
          "type": "object",
          "properties": {
              "name": { "type": "string" },
              "task_id": { "type": "integer" },
              "base_ref": { "type": "string" }
          }, 
          "required": ["name"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_list",
        "description": "List worktrees tracked in .worktrees/index.json.",
        "parameters": { 
          "type": "object", "properties": {} 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_status",
        "description": "Show git status for one worktree.",
        "parameters": { 
          "type": "object",
          "properties": { "name": { "type": "string" } },
          "required": ["name"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_run",
        "description": "Run a shell command in a named worktree directory.",
        "parameters": { "type": "object",
          "properties": {
              "name": { "type": "string" },
              "command": { "type": "string" }
          }, 
          "required": ["name", "command"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_keep",
        "description": "Mark a worktree as kept without removing it.",
        "parameters": { 
          "type": "object",
          "properties": { "name": { "type": "string" } },
          "required": ["name"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_remove",
        "description": "Remove a worktree and optionally complete its bound task.",
        "parameters": { 
        "type": "object",
          "properties": {
              "name": { "type": "string" },
              "force": { "type": "boolean" },
              "complete_task": { "type": "boolean" }
          }, 
          "required": ["name"] 
        }
      }
    },
    { 
      "type": "function", "function": {
        "name": "worktree_events",
        "description": "List recent worktree/task lifecycle events from events.jsonl.",
        "parameters": { 
          "type": "object",
            "properties": { "limit": { "type": "integer" } },
            "properties": {} 
          }
      }
    },
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
  tasks_dir: PathBuf,
  claim_lock: Arc<Mutex<()>>,
) {
  let team_name = { manager.config.lock().unwrap().team_name.clone() };
  let system = format!("you are '{}', role: '{}', team: '{}'. Use idle tool when you have no more work.", name, role, team_name);
  let mut messages: Vec<Message> = vec![Message::User { content: prompt.clone() }];
  // 'outer: loop 开始
  loop {
    // == WORK 阶段 ==
    let mut idle_requested = false;
    let mut should_exit = false;
    for _ in 0..50{
      // 读信箱，遇到 shutdown_request 直接返回
      let inbox_msgs = bus.read_inbox(&name);
      for msg in &inbox_msgs {
        if msg.msg_type == "shutdown_request" {
          manager.set_status(&name, "shutdown");
          return;
        }
        messages.push(Message::User {
          content: serde_json::to_string(msg).unwrap_or_default(),
        });
      }

      // 接下来就是标准的组装请求体
      let mut req_msgs = vec![json!(Message::System { content: system.clone() })];
      for msg in &messages { 
        req_msgs.push(json!(msg)); 
      }
      let body = json!({
        "model": model_id,
        "messages": req_msgs,
        "tools": worktree_tools(),
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
            "shutdown_response" => {
              let req_id = args["request_id"].as_str().unwrap_or("");
              let approve = args["approve"].as_bool().unwrap_or(true);
              // 回复 Lead
              bus.send_with_extra(
                &name,
                "lead",
                if approve { "Shutting down." } else { "Staying alive." },
                "shutdown_response",
                Some(serde_json::json!({
                  "request_id": req_id,
                  "approve": approve,
                })),
              );
              if approve {
                // 设置退出标志（需要在外层加 should_exit bool）
                format!("shutdown_ack:{}:{}", req_id, approve)
              } else {
                format!("Rejected shutdown request {}", req_id)
              }
            },
            "plan_approval" => {
              // Teammate 提交计划模式：只需要 plan 字段
              let plan = args["plan"].as_str().unwrap_or("");
              let req_id = format!(
                "{:x}",
                SystemTime::now()
                  .duration_since(UNIX_EPOCH)
                  .unwrap()
                  .as_millis()
              )[..8].to_string();
              bus.send_with_extra(
                &name,
                "lead",
                plan,
                "plan_approval_request",
                Some(serde_json::json!({
                    "request_id": req_id,
                    "from": name,
                })),
              );
              format!("Plan submitted for approval (req_id: {})", req_id)
            },
            "idle" => {
              idle_requested = true;
              "Entering idle phase. Will poll for new tasks.".to_string()
            },
            "claim_task" => {
              let task_id = args["task_id"].as_u64().unwrap_or(0);
              claim_task(&tasks_dir, task_id, &name, &claim_lock)
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
      if messages.iter().rev().any(|m| {
        if let Message::Tool { content, .. } = m {
          content.starts_with("shutdown_ack:") &&
          content.ends_with(":true")
        } else {
          false
        }
      }) {
        should_exit = true;
      }

      if should_exit {
        break;
      }

      if idle_requested {
        break;
      }
    }
    // == IDLE 阶段 ==
    if should_exit {
      manager.set_status(&name, "shutdown");
      return;
    }
    manager.set_status(&name, "idle");
    let mut resume = false;
    for _ in 0..(IDLE_TIMEOUT / POLL_INTERVAL) {
      sleep(Duration::from_secs(POLL_INTERVAL)).await;

      // 检查信箱
      let inbox = bus.read_inbox(&name);
      if !inbox.is_empty() {
        for msg in &inbox {
          if msg.msg_type == "shutdown_request" {
            manager.set_status(&name, "shutdown");
            return;
          }
          messages.push(Message::User {
            content: serde_json::to_string(msg).unwrap_or_default(),
          });
        }
        resume = true;
        break;
      }

      // 扫描任务板
      let unclaimed = scan_unclaimed_tasks(&tasks_dir);
      if let Some(task) = unclaimed.first() {
        let task_id = task["id"].as_u64().unwrap_or(0);
        let result = claim_task(&tasks_dir, task_id, &name, &claim_lock);
        if !result.starts_with("Error:") {
          if messages.len() <= 3 {
            messages.insert(0, make_identity_block(&name, &role, &team_name));
            messages.insert(1, Message::Assistant {
              content: Some(format!("I am {}. Continuing.", name)),
              tool_calls: None,
            });
          }
          let subject = task["subject"].as_str().unwrap_or("").to_string();
          messages.push(Message::User {
            content: format!("<auto-claimed>Task #{}: {}</auto-claimed>", task_id, subject),
          });
          resume = true;
          break;
        }
      }
    }

    if !resume {
      manager.set_status(&name, "shutdown");
      return;
    }
    manager.set_status(&name, "working");
  } // 'outer: loop 结束
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
  let shutdown_trackers: Arc<Mutex<HashMap<String, ShutdownTracker>>> = 
    Arc::new(Mutex::new(HashMap::new()));
  let plan_trackers: Arc<Mutex<HashMap<String, PlanTracker>>> = Arc::new(Mutex::new(HashMap::new()));
  let manager = TeammateManager::new(team_dir);
  let tasks_dir = env::current_dir()?.join(".tasks");
  let claim_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
  let workdir = env::current_dir()?;
  let task_manager = TaskManager::new(workdir.join(".tasks"));
  let event_bus = EventBus::new(workdir.join(".worktrees").join("events.jsonl"));
  let worktree_manager = WorktreeManager::new(workdir.clone(), task_manager.clone(), event_bus.clone());


  println!("Worktree Task Isolation (s12) Ready");
  let system = format!(
    "You are a coding agent at {:?}. Use task + worktree tools for multi-task work. \
     For parallel or risky changes: create tasks, allocate worktree lanes, \
     run commands in those lanes, then choose keep/remove for closeout. \
     Use worktree_events when you need lifecycle visibility.",
    workdir
);
  let mut messages: Vec<Message> = vec![];
  let mut input = String::new();

  loop {
    // 2: 老板读取信箱
    let msgs = bus.read_inbox("lead");
    for m in msgs {
      println!("\n📬 收到小弟 [{}] 的来信:\n{}\n", m.from, m.content);
    }
    print!("\ns12 >> ");
    std::io::stdout().flush()?;
    input.clear();
    std::io::stdin().read_line(&mut input)?;
    let query = input.trim();
    if query.is_empty() || query == "q" {
      break;
    }

    // 斜线命令
    if query == "/team" {
      println!("{}", manager.list_members());
      continue;
    }
    if query == "/inbox" {
      let msgs = bus.read_inbox("lead");
      println!("{}", serde_json::to_string_pretty(&msgs).unwrap_or_default());
      continue;
    }
    if query == "/tasks" {
      fs::create_dir_all(&tasks_dir).unwrap_or_default();
      if let Ok(entries) = fs::read_dir(&tasks_dir) {
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok())
          .filter(|e| e.file_name().to_string_lossy().starts_with("task_"))
          .collect();
        paths.sort_by_key(|e| e.file_name());
        for entry in paths {
          if let Ok(text) = fs::read_to_string(entry.path()) {
            if let Ok(t) = serde_json::from_str::<Value> (&text) {
              let marker = match t["status"].as_str() {
                Some("pending") => "[ ]",
                Some("in_progress") => "[>]",
                Some("completed") => "[x]",
                _ => "[?]",
              };
              let owner = if t["owner"].is_null() {
                "".to_string()
              } else {
                format!(" @{}", t["owner"].as_str().unwrap_or(""))
              };
              println!("  {} #{}: {}{}", marker, t["id"], t["subject"].as_str().unwrap_or(""), owner);
            }
          }
        }
      }
      continue;
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
        "tools": worktree_tools()
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
              
              // 先把成员加入花名册（如果不存在）
              {
                let mut cfg = manager.config.lock().unwrap();
                if !cfg.members.iter().any(|m| m.name == teammate_name) {
                  cfg.members.push(Teammate {
                    name: teammate_name.clone(),
                    role: teammate_role.clone(),
                    status: "working".to_string(),
                  });
                }
              }
              manager.save_config();

              // 2. 把花名册里这个小弟的状态改为干活中
              manager.set_status(&teammate_name, "working");
  
              // 3. 克隆各种依赖，因为把闭包扔进后台线程后，它会拥有这些变量的所有权
              let c_client = client.clone();
              let c_api = api_key.clone();
              let c_base = base_url.clone();
              let c_model = model_id.clone();
              let c_bus = bus.clone();
              let c_manager = manager.clone();
              let c_tasks = tasks_dir.clone();
              let c_lock = claim_lock.clone();
  
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
                  c_manager,
                  c_tasks,
                  c_lock,
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
            "shutdown_request" => {
              let teammate = args["teammate"].as_str().unwrap_or("");
              handle_shutdown_request(teammate, &bus, &shutdown_trackers)
            },
            "plan_approval" => {
              // Lead 审批模式：需要 request_id + approve
              let req_id = args["request_id"].as_str().unwrap_or("");
              let approve = args["approve"].as_bool().unwrap_or(false);
              let feedback = args["feedback"].as_str().unwrap_or("OK");
              let to = args["to"].as_str().unwrap_or("");
              handle_plan_review(req_id, approve, feedback, to, &bus, &plan_trackers)
            },
            "shutdown_response" => {
              // Lead 侧：查询某个 shutdown 请求的当前状态
              let req_id = args["request_id"].as_str().unwrap_or("");
              let map = shutdown_trackers.lock().unwrap();
              match map.get(req_id) {
                Some(tracker) => format!(
                  "shutdown request {} → target: {}, status: {:?}",
                  req_id, tracker.target, tracker.status
                ),
                None => format!("Error: Unknown request_id '{}'", req_id)
              }
            },
            "idle" => {
              "Lead does not idle.".to_string()
            },
            "claim_task" => {
              let task_id = args["task_id"].as_u64().unwrap_or(0);
              claim_task(&tasks_dir, task_id, "lead", &claim_lock)
            },
            "task_create" => {
              let subject = args["subject"].as_str().unwrap_or("");
              let desc = args["description"].as_str().unwrap_or("");
              task_manager.create(subject, desc)
            },
            "task_list" => task_manager.list_all(),
            "task_get" => {
              let id = args["task_id"].as_u64().unwrap_or(0);
              task_manager.get(id)
            },
            "task_update" => {
              let id = args["task_id"].as_u64().unwrap_or(0);
              let status = args["status"].as_str();
              let owner = args["owner"].as_str();
              task_manager.update(id, status, owner)
            },
            "task_bind_worktree" => {
              let id = args["task_id"].as_u64().unwrap_or(0);
              let wt = args["worktree"].as_str().unwrap_or("");
              task_manager.bind_worktree(id, wt)
            },
            "worktree_create" => {
              let name = args["name"].as_str().unwrap_or("");
              let task_id = args["task_id"].as_u64();
              let base_ref = args["base_ref"].as_str().unwrap_or("HEAD");
              worktree_manager.create(name, task_id, base_ref)
            },
            "worktree_list" => worktree_manager.list_all(),
            "worktree_status" => {
              let name = args["name"].as_str().unwrap_or("");
              worktree_manager.status(name)
            },
            "worktree_run" => {
              let name = args["name"].as_str().unwrap_or("");
              let cmd = args["command"].as_str().unwrap_or("");
              worktree_manager.run_in(name, cmd)
            },
            "worktree_keep" => {
              let name = args["name"].as_str().unwrap_or("");
              worktree_manager.keep(name)
            },
            "worktree_remove" => {
              let name = args["name"].as_str().unwrap_or("");
              let force = args["force"].as_bool().unwrap_or(false);
              let complete = args["complete_task"].as_bool().unwrap_or(false);
              worktree_manager.remove(name, force, complete)
            },
            "worktree_events" => {
              let limit = args["limit"].as_u64().unwrap_or(20) as usize;
              event_bus.list_recent(limit)
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
