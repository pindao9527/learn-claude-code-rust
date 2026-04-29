# s09: Agent Teams (Agent 团队)

`s01 > s02 > s03 > s04 > s05 > s06 | s07 > s08 > [ s09 ] s10 > s11 > s12`

> *"任务太大一个人干不完, 要能分给队友"* —— 持久化队友 + JSONL 邮箱。
>
> **Harness 层**: 团队邮箱 —— 多个模型，通过文件协调（引入跨进程排他锁）。

## 问题

Subagent (s04) 是一次性的：生成、干活、返回摘要、消亡。没有身份，没有跨调用的记忆。Background Tasks (s08) 能跑 shell 命令，但做不了 LLM 引导的决策。

真正的团队协作需要三样东西：(1) 能跨多轮对话存活的持久 Agent，(2) 身份和生命周期管理，(3) Agent 之间的通信通道。

## 解决方案

```text
Teammate lifecycle:
  spawn -> WORKING -> IDLE -> WORKING -> ... -> SHUTDOWN

Communication:
  .team/
    config.json           <- team roster + statuses (Shared via Arc<Mutex<T>>)
    inbox/
      alice.jsonl         <- append-only, drain-on-read (Exclusive file lock)
      bob.jsonl
      lead.jsonl

              +--------+    send("alice","bob","...")    +--------+
              | alice  | -----------------------------> |  bob   |
              | loop   |    bob.jsonl << {json_line}    |  loop  |
              +--------+                                +--------+
                   ^                                         |
                   |        BUS.read_inbox("alice")          |
                   +---- alice.jsonl -> read + drain ---------+
```

## 工作原理

1. **TeammateManager**: 通过 `config.json` 维护团队名册。在 Rust 中，我们使用 `Arc<Mutex<TeamConfig>>` 确保跨线程修改状态时的原子性。

```rust
pub struct TeammateManager {
    pub dir: PathBuf,
    pub config: Arc<Mutex<TeamConfig>>, // 线程安全共享状态
}

impl TeammateManager {
    pub fn set_status(&self, name: &str, new_status: &str) {
        let mut cfg = self.config.lock().unwrap();
        if let Some(m) = cfg.members.iter_mut().find(|m| m.name == name) {
            m.status = new_status.to_string();
        }
        self.save_config(&cfg);
    }
}
```

2. **`spawn()`**: 创建队友并在 Tokio 异步任务中启动 agent loop。

```rust
// 在 Lead Agent 的 loop 中
tokio::spawn(async move {
    _teammate_loop(
        name, role, prompt, 
        c_client, c_api, c_base, c_model, c_bus, c_manager
    ).await;
});
```

3. **MessageBus**: 带文件锁的 JSONL 收件箱。使用 `fs2` 保证跨进程安全。

```rust
impl MessageBus {
    pub fn send(&self, from: &str, to: &str, content: &str, msg_type: &str) {
        let path = self.dir.join(format!("{}.jsonl", to));
        let mut file = OpenOptions::new().append(true).create(true).open(path).unwrap();
        
        file.lock_exclusive().unwrap(); // 关键：获取文件排他锁
        writeln!(file, "{}", serde_json::to_string(&msg).unwrap()).unwrap();
        file.unlock().unwrap();
    }

    pub fn read_inbox(&self, name: &str) -> Vec<InboxMessage> {
        let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
        file.lock_exclusive().unwrap();
        // 读取内容...
        file.set_len(0).unwrap(); // 关键：原子清空收件箱
        file.unlock().unwrap();
        // 返回解析后的消息列表
    }
}
```

4. **_teammate_loop**: 每个队友在每次 LLM 调用前检查收件箱，实现“消息驱动”。

```rust
async fn _teammate_loop(...) {
    loop {
        let inbox_msgs = bus.read_inbox(&name);
        if !inbox_msgs.is_empty() {
            messages.push(Message::User { content: format_inbox(inbox_msgs) });
        }
        
        // 发起 LLM 请求并处理工具调用...
    }
}
```

## 相对 s08 的变更

| 组件           | 之前 (s08)       | 之后 (s09)                         |
|----------------|------------------|------------------------------------|
| Tools          | 6                | 9 (+spawn/send/read_inbox/list/broadcast) |
| Agent 数量     | 单一             | 领导 + N 个队友                    |
| 持久化         | 无               | config.json + JSONL 收件箱         |
| 任务调度       | 后台命令         | `tokio::spawn` 独立 Agent 循环     |
| 生命周期       | 一次性           | working -> idle -> working         |
| 通信机制       | 无               | 文件锁保护的消息队列 (MessageBus)  |

## 试一试

```sh
cd learn-claude-code-rust
cargo run --bin s09
```

试试这些指令：

1. `Spawn alice (coder) and bob (tester). Have alice send bob a message.`
2. `Broadcast "status update: phase 1 complete" to all teammates`
3. `Check the lead inbox for any messages`
4. 询问：“看看现在有哪些队员都在干什么？” (调用 `list_teammates`)
