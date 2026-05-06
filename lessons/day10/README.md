# Day 10：团队协议 (Team Protocols)

> *"同一个 request_id，两种握手协议"* —— 用状态机建模 shutdown 和 plan-approval 双向确认流程。

## 1. 目标

在 Day 09 中，我们建立了 Lead + Teammate 的通信管道（MessageBus + JSONL 信箱）。
今天我们在这条管道上实现两套**结构化握手协议**：

| 协议 | 发起方 | 响应方 | 结果 |
|------|-------|-------|------|
| **Shutdown** | Lead | Teammate | Teammate 停止运行 |
| **Plan Approval** | Teammate | Lead | Lead 批准/拒绝计划 |

两套协议共享同一个核心模式：**request_id 关联 + 状态追踪**。

---

## 2. 状态机 (State Machine) 核心概念

### 为什么用 enum 建模状态？

```rust
// ❌ Python 风格：字符串状态（运行时才能发现错误）
tracker["status"] = "approvd"  // 拼写错误！运行时才崩溃

// ✅ Rust 风格：enum 状态（编译时就能发现错误）
enum RequestStatus {
    Pending,
    Approved,
    Rejected,
}
// match 必须穷举所有分支，漏掉任何一个 → 编译报错
```

### 状态转换图

```
Shutdown FSM:
  Pending --> [收到 approve=true]  --> Approved
  Pending --> [收到 approve=false] --> Rejected

Plan Approval FSM:
  Pending --> [Lead approve=true]  --> Approved
  Pending --> [Lead approve=false] --> Rejected
```

### enum 附带数据 (Enum with Data)

Rust 的 enum 变体可以携带数据，这是 Python 做不到的：

```rust
// 一个 enum 可以描述完整的请求记录！
enum ShutdownRequest {
    Pending { target: String, req_id: String },
    Approved { target: String, req_id: String },
    Rejected { target: String, req_id: String, reason: String },
}
```

---

## 3. 两套协议的 Claude Code 逻辑

### 3.1 Shutdown 协议（Lead 主动关机）

```
Lead 侧：
  1. 生成唯一 request_id (UUID 前8位)
  2. 写入 shutdown_requests 追踪器 { req_id: { target, status: Pending } }
  3. 通过 MessageBus 发送 shutdown_request 消息给 Teammate
  4. 等待 Teammate 回复（通过 read_inbox 轮询）

Teammate 侧：
  1. 在每轮 LLM 调用前读取信箱
  2. 发现 shutdown_request 消息
  3. LLM 决策：调用 shutdown_response 工具（approve=true/false）
  4. 发送 shutdown_response 消息回 Lead（携带相同 request_id）
  5. 如果 approve=true，退出循环

Lead 侧（收到回复后）：
  1. 通过 request_id 在追踪器里更新状态 Pending → Approved/Rejected
```

### 3.2 Plan Approval 协议（Teammate 申请审批）

```
Teammate 侧：
  1. 生成 request_id
  2. 调用 plan_approval 工具（提交计划文本）
  3. 发送 plan_approval_response 消息给 Lead

Lead 侧：
  1. 从信箱读取 plan_approval_response 消息
  2. LLM 决策：调用 plan_approval 工具（request_id + approve）
  3. 发送 plan_approval_response 消息回 Teammate

Teammate 侧（收到回复后）：
  1. 检查 approve 字段
  2. 继续执行（approved）或中止（rejected）
```

---

## 4. 今日编码任务（手动实现顺序）

### 第一步：定义状态机 enum

```rust
// 在文件顶部新增
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
```

### 第二步：InboxMessage 增加 extra 字段

现有的 InboxMessage 没有 `request_id`、`approve` 等字段。
需要增加一个灵活的 `extra` 字段来承载协议元数据：

```rust
pub struct InboxMessage {
    pub msg_type: String,
    pub from: String,
    pub content: String,
    pub timestamp: u64,
    // 新增：用 serde_json::Value 承载任意额外字段
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}
```

### 第三步：MessageBus::send 支持 extra

```rust
pub fn send_with_extra(&self, sender: &str, to: &str, content: &str, 
                        msg_type: &str, extra: Option<serde_json::Value>) {
    // 构建消息时带上 extra 字段
}
```

### 第四步：新增协议工具处理函数

```rust
// Lead 发出 shutdown 请求
fn handle_shutdown_request(teammate: &str, bus: &MessageBus, 
                            trackers: &Arc<Mutex<HashMap<String, ShutdownTracker>>>) -> String

// Lead 审批 plan
fn handle_plan_review(req_id: &str, approve: bool, feedback: &str,
                       bus: &MessageBus,
                       plan_trackers: &Arc<Mutex<HashMap<String, PlanTracker>>>) -> String
```

### 第五步：更新工具列表

Lead 的工具列表新增：`shutdown_request`, `shutdown_response`(查状态), `plan_approval`(审批)
Teammate 的工具列表新增：`shutdown_response`(回应), `plan_approval`(提交计划)

### 第六步：更新 _teammate_loop

- 处理 `shutdown_response` 工具调用（设置 should_exit = true）
- 处理 `plan_approval` 工具调用（提交计划）

---

## 5. Rust 知识点清单

| 概念 | 用途 | 本节示例 |
|------|------|---------|
| `enum` 状态机 | 类型安全的状态建模 | `RequestStatus::Pending/Approved/Rejected` |
| `HashMap<K, V>` | 按 request_id 追踪请求 | `shutdown_requests: HashMap<String, ShutdownTracker>` |
| `Arc<Mutex<HashMap>>` | 跨线程共享追踪器 | Lead 主线程和 Teammate 后台线程共享状态 |
| `match` 穷举 | 处理状态转换 | `match status { Pending => ..., Approved => ..., Rejected => ... }` |
| `uuid` 短 ID | 生成唯一 request_id | `format!("{}", uuid::Uuid::new_v4())[..8]` |

---

## 6. 关键理解：为什么 request_id 这么重要？

```
问题：Lead 发出了 3 个 shutdown 请求（alice, bob, charlie），
      Teammate 返回的 shutdown_response 怎么知道是回复哪个的？

答案：request_id！
  Lead 发出时：{ request_id: "abc123", target: "alice" }
  Alice 回复时：{ request_id: "abc123", approve: true }
  Lead 查表时：shutdown_requests["abc123"] → { target: "alice", status: Approved }
```

这就是**关联（Correlation）模式**，也是所有异步系统的核心设计！
