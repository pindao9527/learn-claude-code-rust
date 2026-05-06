# Day 10: Review & Key Concepts

## 1. 状态机：enum 的真正力量

Python 用字符串 `"pending"` / `"approved"` 表示状态，拼写错误只能在运行时才发现。
Rust 的 `enum` 让状态机变成编译时保证：

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum RequestStatus {
    Pending,
    Approved,
    Rejected,
}
```

- **穷举检查**：`match` 必须处理所有变体，漏掉任何一个 → 编译报错。
- **`PartialEq`**：通过 `derive` 自动获得 `==` 比较能力，无需手写。
- **`Debug`**：`{:?}` 格式化时可以打印出枚举名称，调试极其方便。

## 2. request_id 关联模式（Correlation Pattern）

异步系统里，"请求"和"回复"分属不同时刻、不同线程。用 `request_id` 把它们串起来：

```rust
// 发请求时：生成 ID，写入追踪器
let req_id = format!("{:x}", SystemTime::now()
    .duration_since(UNIX_EPOCH).unwrap().as_millis())[..8].to_string();
shutdown_trackers.lock().unwrap().insert(req_id.clone(), ShutdownTracker {
    target: teammate.to_string(),
    status: RequestStatus::Pending,
});

// 收回复时：用同一个 ID 查表，更新状态
let map = shutdown_trackers.lock().unwrap();
match map.get(req_id) {
    Some(tracker) => /* 找到了，更新状态 */,
    None => /* 未知 ID，报错 */,
}
```

这个模式是所有分布式/异步系统的基石，RPC、HTTP 幂等请求、消息队列都在用它。

## 3. Arc<Mutex<HashMap>> 跨线程追踪器

Lead 主线程和多个 Teammate 后台线程需要同时访问追踪器：

```rust
let shutdown_trackers: Arc<Mutex<HashMap<String, ShutdownTracker>>> =
    Arc::new(Mutex::new(HashMap::new()));
```

- **`Arc`**：引用计数，让多个线程"共同拥有"同一份数据。
- **`Mutex`**：同一时刻只有一个线程能写，防止数据竞争。
- **锁要尽早释放**：用 `{ let mut map = trackers.lock().unwrap(); ... }` 包一个代码块，块结束时锁自动释放，避免死锁。

## 4. should_exit 标志：协议驱动的生命周期

Teammate 不是被"杀死"，而是自愿退出——这才是真正的优雅关机：

```rust
let mut should_exit = false;
for _ in 0..30 {
    // ... 正常工作 ...
    // 检查是否收到 shutdown_ack
    if messages.iter().rev().any(|m| matches!(m,
        Message::Tool { content, .. }
        if content.starts_with("shutdown_ack:") && content.ends_with(":true")
    )) {
        should_exit = true;
    }
    if should_exit { break; }
}
manager.set_status(&name, if should_exit { "shutdown" } else { "idle" });
```

- 状态写回花名册，Lead 可以随时查询哪些 Teammate 已关机。
- `"shutdown"` 和 `"idle"` 是两种不同的正常退出状态。

## 5. #[serde(default)]：向后兼容

给已有结构体新增字段时，旧的 JSONL 文件里没有这个字段，反序列化会失败。

```rust
#[serde(default)]
pub extra: Option<serde_json::Value>,
```

`#[serde(default)]` 告诉 serde：字段缺失时，用 `Default::default()`（即 `None`）填充，而不是报错。这是协议演化的标准做法。

## 💡 课后挑战

试着把 `request_id` 的生成从"时间戳截断"改为真正的 UUID：

```rust
// Cargo.toml 里已经有 uuid = { version = "1", features = ["v4"] }
use uuid::Uuid;
let req_id = &Uuid::new_v4().to_string()[..8];
```

观察一下：为什么时间戳截断在并发场景下可能产生重复 ID，而 UUID v4 不会？
