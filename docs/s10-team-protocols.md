# s10: Team Protocols (团队协议)

`s01 > s02 > s03 > s04 > s05 > s06 | s07 > s08 > s09 > [ s10 ] s11 > s12`

> *"队友之间要有统一的沟通规矩"* -- 一个 request-response 模式驱动所有协商。
>
> **Harness 层**: 协议 -- 模型之间的结构化握手。

## 问题

s09 中队友能干活能通信, 但缺少结构化协调:

**关机**: 直接杀线程会留下写了一半的文件和过期的 config.json。需要握手 -- 领导请求, 队友批准 (收尾退出) 或拒绝 (继续干)。

**计划审批**: 领导说 "重构认证模块", 队友立刻开干。高风险变更应该先过审。

两者结构一样: 一方发带唯一 ID 的请求, 另一方引用同一 ID 响应。

## 解决方案

```
Shutdown Protocol            Plan Approval Protocol
==================           ======================

Lead             Teammate    Teammate           Lead
  |                 |           |                 |
  |--shutdown_req-->|           |--plan_req------>|
  | {req_id:"abc"}  |           | {req_id:"xyz"}  |
  |                 |           |                 |
  |<--shutdown_resp-|           |<--plan_resp-----|
  | {req_id:"abc",  |           | {req_id:"xyz",  |
  |  approve:true}  |           |  approve:true}  |

Shared FSM:
  [pending] --approve--> [approved]
  [pending] --reject---> [rejected]

Trackers:
  shutdown_requests = {req_id: {target, status}}
  plan_requests     = {req_id: {from, plan, status}}
```

## 工作原理

1. 领导生成 request_id, 通过收件箱发起关机请求。

```rust
fn handle_shutdown_request(
    teammate: &str,
    bus: &MessageBus,
    trackers: &Arc<Mutex<HashMap<String, ShutdownTracker>>>,
) -> String {
    let req_id = format!("{:x}", SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap().as_millis())[..8].to_string();
    {
        let mut map = trackers.lock().unwrap();
        map.insert(req_id.clone(), ShutdownTracker {
            target: teammate.to_string(),
            status: RequestStatus::Pending,
        });
    }
    bus.send_with_extra("lead", teammate, "Please shut down gracefully.",
        "shutdown_request", Some(json!({ "request_id": req_id })));
    format!("Shutdown request {} sent to '{}' (status: Pending)", req_id, teammate)
}
```

2. 队友收到请求后, 用 approve/reject 响应。同时设置 `should_exit` 让循环真正退出。

```rust
"shutdown_response" => {
    let req_id = args["request_id"].as_str().unwrap_or("");
    let approve = args["approve"].as_bool().unwrap_or(true);
    bus.send_with_extra(&name, "lead",
        if approve { "Shutting down." } else { "Staying alive." },
        "shutdown_response",
        Some(json!({ "request_id": req_id, "approve": approve })));
    if approve {
        format!("shutdown_ack:{}:true", req_id)  // 触发 should_exit
    } else {
        format!("Rejected shutdown request {}", req_id)
    }
},
// 循环末尾检查退出信号
if messages.iter().rev().any(|m| matches!(m,
    Message::Tool { content, .. } if content.starts_with("shutdown_ack:") && content.ends_with(":true")
)) {
    should_exit = true;
}
```

3. 计划审批遵循完全相同的模式。队友提交计划（生成 request_id），领导审查（引用同一个 request_id）。

```rust
fn handle_plan_review(
    req_id: &str, approve: bool, feedback: &str, to: &str,
    bus: &MessageBus,
    plan_trackers: &Arc<Mutex<HashMap<String, PlanTracker>>>,
) -> String {
    {
        let mut map = plan_trackers.lock().unwrap();
        if let Some(tracker) = map.get_mut(req_id) {
            tracker.status = if approve { RequestStatus::Approved } else { RequestStatus::Rejected };
        }
    }
    bus.send_with_extra("lead", to, feedback, "plan_approval_response",
        Some(json!({ "request_id": req_id, "approve": approve })));
    format!("Plan {} (req_id: {}) → {}", to, req_id,
        if approve { "APPROVED" } else { "REJECTED" })
}
```

一个 FSM, 两种用途。同样的 `Pending → Approved | Rejected` 状态机（Rust `enum`）可以套用到任何请求-响应协议上。

## Rust vs Python 关键差异

| 概念 | Python | Rust |
|------|--------|------|
| 状态表示 | 字符串 `"pending"` | `enum RequestStatus { Pending, Approved, Rejected }` |
| 追踪器 | `dict` | `HashMap<String, ShutdownTracker>` |
| 线程共享 | `threading.Lock()` | `Arc<Mutex<HashMap<...>>>` |
| 退出标志 | `should_exit = True` | `let mut should_exit = false;` + `break` |
| extra 字段 | `dict` 直接 merge | `Option<serde_json::Value>` + `#[serde(default)]` |

## 相对 s09 的变更

| 组件           | 之前 (s09)       | 之后 (s10)                           |
|----------------|------------------|--------------------------------------|
| Tools          | 9                | 12 (+shutdown_req/resp +plan)        |
| 关机           | 仅自然退出       | 请求-响应握手                        |
| 计划门控       | 无               | 提交/审查与审批                      |
| 关联           | 无               | 每个请求一个 request_id              |
| FSM            | 无               | `enum RequestStatus` 状态机          |

## 试一试

```sh
cd learn-claude-code-rust
cargo run --bin s10
```

试试这些 prompt：

1. `Spawn alice as a coder. Then request her shutdown.`
2. `List teammates to see alice's status after shutdown approval`
3. `Spawn bob with a risky refactoring task. Review and reject his plan.`
4. `Spawn charlie, have him submit a plan, then approve it.`
