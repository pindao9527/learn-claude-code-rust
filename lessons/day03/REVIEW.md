# Day 03 复习卡：struct & impl

## 1. struct 只管数据，impl 只管行为

```rust
// 数据
struct TodoManager {
    items: Vec<PlanItem>,
    rounds_since_update: u32,
}

// 行为
impl TodoManager {
    fn new() -> Self { ... }
    fn update(&mut self, ...) { ... }
    fn render(&self) -> String { ... }
}
```

Python 把两者写在一个 `class` 里，Rust 强制分开。好处是：数据结构一眼看清，行为单独维护。

## 2. &self / &mut self / 无 self 怎么选

| 签名 | 含义 | 使用场景 |
|------|------|---------|
| `fn new() -> Self` | 无 self，关联函数 | 构造器，不依赖已有实例 |
| `fn render(&self)` | 只读借用 | 只读取字段，不修改 |
| `fn update(&mut self)` | 可变借用 | 需要修改字段 |

判断方法：**这个方法需要修改结构体的字段吗？需要就 `&mut self`，不需要就 `&self`。**

## 3. 默认值写在 new() 里

Rust struct 字段不支持默认值语法，默认值统一在构造函数里设：

```rust
impl PlanItem {
    fn new(content: String) -> Self {
        PlanItem {
            content,
            status: "pending".to_string(),  // 默认值在这里
            active_form: String::new(),
        }
    }
}
```

## 4. Option<T> 表达"可能没有"

```rust
fn reminder(&self) -> Option<String> {
    if self.rounds_since_update < 3 {
        return None;           // 没有提醒
    }
    Some("请更新计划".to_string())  // 有提醒
}

// 调用方
if let Some(r) = todo.reminder() {
    println!("{}", r);
}
```

比返回空字符串更准确，调用方被迫处理"没有"的情况。

## 5. 今天顺手补的安全改进

**路径逃逸检查**：防止 `../../../etc/passwd` 这类攻击
```rust
fn safe_path(p: &str) -> Result<PathBuf, String> {
    let cwd = current_dir()?;
    let path = cwd.join(p);
    if !path.starts_with(&cwd) {
        return Err(format!("Path escapes workspace: {}", p));
    }
    Ok(path)
}
```

**bash 超时**：防止命令挂死阻塞 Agent
```rust
match child.wait_timeout(Duration::from_secs(120)) {
    None => { child.kill(); "Error: Timeout (120s)".to_string() }
    Some(_) => { /* 正常读取输出 */ }
}
```

---

### 💡 第三天心得

`struct + impl` 是 Rust 组织代码的基本单元。今天的 `TodoManager` 就是一个完整的例子：状态内聚、行为清晰、边界明确。下一步 Day 04 会用 `enum + match` 替代字符串比较，让状态流转更安全。
