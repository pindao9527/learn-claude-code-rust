# Day 05 复习卡：技能加载与错误处理原理

## 1. 核心理论：从恐慌走向优雅

在 Day 05 的实战中，我们学到了 Rust 强类型错误处理的核心哲学：**错误不是异类，而是普通的返回值。**

```rust
// ❌ Python风格（暗雷模式）隐式抛出
def do_something():
    content = read_file("skill.md") # 若失败，系统直接崩溃或者被上层不知名的 try-except 拦截

// ✅ Rust风格（阳光模式）显式标注
fn do_something() -> Result<String, SkillError> {
    let content = fs::read_to_string("skill.md")?; 
    Ok(content)
}
```
**`Result<T, E>`** 强迫你和编译器一起面对所有的异常可能性。如果不显式处理（如调用 `.unwrap()` 或使用 `?`），代码根本无法编译。

## 2. 理解神奇的 `?` 操作符

`?` 并不是简单地“报错就退出”。它的完整语义是：**“如果返回 `Ok`，就将值解包并赋值；如果返回 `Err`，就提前 `return`，并尝试将错误自动转换（`From` trait）为你函数签名的错误类型。”**

等价展开：
```rust
// 简写：
let content = fs::read_to_string("skill.md")?;

// 编译器为你展开的面貌：
let content = match fs::read_to_string("skill.md") {
    Ok(val) => val,
    Err(err) => return Err(From::from(err)), // 将 std::io::Error 转化为你的 SkillError
};
```

## 3. 为什么需要 `thiserror`？

因为如果自己实现一个标准的错误枚举，你需要写大量样板代码：
1. 实现 `std::fmt::Display`
2. 实现 `std::fmt::Debug`
3. 实现 `std::error::Error`
4. 为每一个第三方错误（如 `io::Error`）实现 `From` trait。

而使用 `thiserror`，过程宏（macro）帮你把这一切压缩成了极具表达力的代码：

```rust
#[derive(Error, Debug)]
pub enum SkillError {
    #[error("文件读取失败: {0}")]
    Io(#[from] std::io::Error), // 自动帮你实现了 From<std::io::Error>

    #[error("未找到对应名称的技能: {0}")]
    NotFound(String), // 自定义领域错误
}
```

## 4. 两层技能模型

为什么要有 `SkillManifest` 和 `SkillDocument` 两个结构体？
因为你的 **System Prompt** 容量有限，而且不能在一开始就把几万字的专业技能全部暴露给模型。

- **`SkillManifest`**：简历阶段，只有简短的内容 `name` 和 `description`，用于拼接出给模型看的 `Available skills` 清单。
- **`SkillDocument`**：正式工作，只有在大模型说出 `load_skill("pdf")` 的时候，才临时将重磅的 `body` 内容传回上下文，实现了按需加载与 Token 的极致节省。

## 5. 设计反思：传递引用的魅力

在 Python 中，`SKILL_REGISTRY` 是一个直接挂载在模块顶部的全局变量。
在 Rust 中，我们体验了更好的形式：**依赖注入与显式引用传递**。

```rust
// 在 main 初始化
let registry = SkillRegistry::new(pwd);

// 一级级向下借出引用
agent_loop(..., &registry).await;
run_subagent(..., registry).await; // 注意传参顺序
```

我们无需使用 `lazy_static` 这类引入成本极高的单例模式，仅靠一个借阅证 `&registry` 就轻盈地打通了多级调用的壁垒。这也为后续的并发安全环境奠定了干净的基础。

---

### 💡 第五天心得

`unwrap()` 代表着一种偷懒和推诿，而基于 `thiserror` 定义出严谨的 `pub enum MyError` 则体现了一名成熟 Rustaceans 的修养。当你设计出合理的 Error 枚举后，你会发现在函数里用 `?` 来组合不同类型底层方法的执行，就如同拼装积木般流畅和令人安心。
