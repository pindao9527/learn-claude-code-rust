# Day 03：封装的力量 —— struct & impl

对应原版：`s03_todo_write.py`

## 今天做什么

在 `s02` 的基础上新增一个 `TodoManager` 结构体，让 Agent 具备会话规划能力。

工作流程：
1. 把 `s02_tool_use.rs` 复制一份，重命名为 `s03_todo_write.rs`
2. 新增 `PlanItem` 和 `TodoManager` 两个结构体
3. 给 `TodoManager` 实现方法（`impl`）
4. 注册 `todo` 工具，接入 Agent 循环

---

## 学习目标

1. 理解 `struct` 如何聚合相关状态（对比 Python `@dataclass`）
2. 掌握 `impl` 块中 `&self` / `&mut self` 的选择逻辑
3. 理解关联函数 `new()` 与普通方法的区别
4. 顺手补上 `safe_path` 的路径逃逸安全检查

---

## 核心概念：struct vs impl

Python 把数据和行为写在一起：

```python
@dataclass
class TodoManager:
    def __init__(self):
        self.state = PlanningState()

    def update(self, items):  # 行为也在 class 里
        ...
```

Rust 把数据和行为**分开定义**：

```rust
// 只管数据
struct TodoManager {
    items: Vec<PlanItem>,
    rounds_since_update: u32,
}

// 只管行为
impl TodoManager {
    fn new() -> Self { ... }          // 关联函数，无 self
    fn update(&mut self, ...) { ... } // 可变方法，需要修改字段
    fn render(&self) -> String { ... } // 只读方法
}
```

**`&self` vs `&mut self` 怎么选？**

问自己一个问题：这个方法需要修改结构体的字段吗？
- 需要 → `&mut self`
- 不需要 → `&self`

---

## 今天要实现的结构体

### PlanItem

```rust
struct PlanItem {
    content: String,
    status: String,      // "pending" | "in_progress" | "completed"
    active_form: String,
}
```

### TodoManager

```rust
struct TodoManager {
    items: Vec<PlanItem>,
    rounds_since_update: u32,
}
```

需要实现的方法：

| 方法 | self 类型 | 作用 |
|------|-----------|------|
| `new()` | 无 | 构造空的 TodoManager |
| `update(&mut self, items: &Value) -> String` | `&mut self` | 更新任务列表，返回渲染结果 |
| `note_round(&mut self)` | `&mut self` | 记录一轮未更新 |
| `reminder(&self) -> Option<String>` | `&self` | 超过3轮未更新时返回提醒 |
| `render(&self) -> String` | `&self` | 把任务列表渲染成文本 |

---

## 思考题（写完后回答）

1. `TodoManager::new()` 为什么不需要 `&self`？
2. `update()` 为什么需要 `&mut self`，而 `render()` 只需要 `&self`？
3. Python 用 `str = "pending"` 设置默认值，Rust 里怎么实现？

---

## 完成标准

- `cargo check` 无报错
- `cargo run --bin s03` 能正常启动
- 给 Agent 布置多步任务时，能看到 `todo` 工具被调用，任务列表被打印出来
