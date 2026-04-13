# Day 05：优雅的错误处理与技能加载 (Result, ?, 与 thiserror)

对应原版：`s05_skill_loading.py`

## 今天做什么

在 `s04` 的子智能体架构之上，今天我们的主要目标是引入**“动态加载特定领域知识”（Skill Loading）**的能力，同时重点突破 Rust 开发中最重要的一环：**错误处理体制**。

工作流程：
1. 更新 `Cargo.toml` 增加 `thiserror` 和 `walkdir` 依赖。
2. 将原版的 `skills` 目录结构搬运过来。
3. 从 `s04_subagent.rs` 复制起步，并重命名为 `s05_skill_loading.rs`。
4. 定义自有的业务错误枚举 `SkillError`（重点！）。
5. 编写数据结构 `SkillManifest` 和 `SkillDocument`。
6. 实现 `SkillRegistry` 读取目录、解析 Frontmatter。
7. 给父子智能体新增 `load_skill` 工具以加载具体的 Markdown 内容。

---

## 学习目标

1. 告别 `unwrap()` 和随意的字符串错误返回，理解强类型的 `Result<T, E>` 如何约束程序的健壮性。
2. 掌握 `?` 操作符在内部实现上的原理，即“错误向上冒泡”。
3. 掌握工程级应用中，如何基于第三方包 `thiserror` 快速通过宏（Macro）生成实现完整 `Error` trait 的领域特定错误枚举（Domain Error）。
4. 学习基础的字符串解析逻辑（Frontmatter 截取）。

---

## 核心概念：为何错误处理如此不同？

### Python 的做法：暗雷与全局异常
在 Python 版本的 `s05_skill_loading.py` 里，如果我们读取文件失败，或是解析报错：
```python
try:
    ...
except Exception as exc:
    return f"Error: {exc}"
```
它能捕捉所有异常，如果漏掉了 `try-catch` 代码本身逻辑还能跑，直到触发 bug 炸掉你的控制流。

### Rust 的做法：阳光下的 Result 与问号
在 Rust 中没有异常 (Exceptions)。所有的错误都是以普通的“值”形式被显式返回。
```rust
fn do_something() -> Result<String, SkillError> {
    // std::fs::read_to_string 返回的是 Result<String, std::io::Error>
    // 在变量后加上 `?`，表示如果发生 Error，直接将这个 Error Return 回去。
    let file = std::fs::read_to_string("skill.md")?; 
    
    // 如果走到这里，说明 file 一定是读取成功的 String 数据
    Ok(file)
}
```

为了让这套流程走得通（让你底层的 `std::io::Error` 能无缝转换成你函数的返回类型 `SkillError`），我们就需要大显身手的 `thiserror`：

```rust
#[derive(Error, Debug)]
pub enum SkillError {
    #[error("文件读取失败: {0}")]
    Io(#[from] std::io::Error), // #[from] 让 `?` 操作符自动把 io::Error 转换成 SkillError::Io
}
```

---

## 今天要实现的结构

### 数据模型
主要为了隔离元数据（给大模型看有哪些技能）和正文（大模型确认加载时才发过去，省 Token）：
```rust
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

pub struct SkillDocument {
    pub manifest: SkillManifest,
    pub body: String,
}
```

### 核心引擎
```rust
struct SkillRegistry {
    skills_dir: PathBuf,
    documents: HashMap<String, SkillDocument>,
}
```

---

## 思考题（写完后回答）

1. 在使用 `?` 操作符把 `std::io::Error` 冒泡转换为 `SkillError` 的过程中，如果不使用 `thiserror` 的 `#[from]`，我们需要手动实现哪个 Trait 才能达到同样的效果？
2. 在返回给工具调用的最终结果（字符串）时，大模型在乎错误类型是不是强类型的 `SkillError` 吗？如果不在乎，为什么我们要在 Rust 引擎内部搞得这么严格？

---

## 完成标准

- `cargo check` 无报错。
- `cargo run --bin s05` 能正常启动。
- 系统提示词（System Prompt）中能够正确通过 `describe_available()` 注入 `skills` 目录下的可用技能名称和描述。
- 你对 Agent 发出请求："请你查一下 react 相关的技能指导"，大模型能够自行调用 `load_skill` 并把读取到的正文运用于之后的任务分析中。
