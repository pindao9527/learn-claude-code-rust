# Day 06：上下文压缩 (Context Compact) 与 Trait/泛型抽象

对应原版：`s06_context_compact.py`

## 今天做什么

当你的 Agent 处理一个长期对话，特别是在进行复杂的工具调用和大段结果读取时，对话历史（Message History）会变得越来越长。这不仅会消耗过度的 Token（导致请求变慢/变贵），还可能超过大语言模型本身的 Context Window（上下文限制），导致报错。

在原版的 Python 案例中，作者使用了硬编码的函数处理这些事情：比如 `micro_compact` 剔除老的工具大片输出，或者 `compact_history` 来调用 LLM 总结历史并抛弃原有的全量长对话。

而在 Rust 中，为了让各种特定的策略解耦，展现更加工程化的一面，我们将学习**Trait (特性/接口)** 与 **泛型 (Generics)**。

工作流程：
1. 复制一份 `s05` 的基础代码作为起步点，并更新 `Cargo.toml`。
2. 定义一个通用的 `Compressor` Trait，建立所有压缩规则都必须遵守的契约。
3. 实现 `MicroCompressor`：体会 Rust 中直接借用（`&mut`）与模式匹配在处理不可预测长度集合并**就地修改数据**上的无敌表现。
4. 实现 `SummaryCompressor`：编写一个异步的 Trait 方法，使之带状态并发起大模型总结请求。
5. 融入到 `agent_loop`：把定义好的 Trait 挂载到我们原本的执行流上。

---

## 学习目标

1. 掌握 **Trait（特性）** 的概念。相比面向对象的基类（Base Class），Trait 更像是纯粹的行为契约（Interface）。
2. 在接口中返回异步的 `Future`：体验 Rust 处理 `async fn in trait` 后的一些兼容性写法（也就是 `Pin<Box<dyn Future>>` 的应用），虽然稍显啰嗦，却是底层并发不可不学的部分。
3. 加深对于 `&mut`（可变引用借用）和生命周期 `'a` 协同工作的理解。
4. 解锁泛型 `T` 或 `Box<dyn Trait>` 这种类型擦除的知识，明白**静态分发（Static Dispatch）**与**动态分发（Dynamic Dispatch）**。

---

## 核心概念：为何抽象如此重要？

### 扩展性（Open-Closed Principle）
如果之后我们想增加一个名为 “过滤无效对话的压缩器”：
- **硬编码方式**：我们需要去 `agent_loop` 里写第 3 个 `if` 和第 3 个函数调用。
- **Trait 结合方式**：只需要写一个新的 `WeedCompressor` 实现了 `Compressor`，并在启动时塞入压缩器列表里即可。原来的主逻辑代码**不需要修改任何一行**！

### 泛型（Generics）与 Trait Object
如果你想声明一个拥有压缩器的 Agent 结构：
```rust
// 方式 A (动态分发 Trait Object): 
// 大小在编译期未知（放在堆里的 Box），但在运行时可以将各种各样不同的实现了特性的结构体塞进同一个 Vec 中。
struct Agent {
    compressors: Vec<Box<dyn Compressor>>,
}

// 方式 B (静态分发 Generics 泛型):
// 大小和类型在编译期完全确定，运行速度最快，但只能容纳一种指定的类型 C。
struct Agent<C: Compressor> {
    compressor: C,
}
```
今天我们会在实战中接触他们。

---

## 思考题（写完后回答）

1. 在 `MicroCompressor` 的实现里，我们只用了借用 `&mut Vec<Message>` 而直接用 `*content = ...` 修改了字符串。如果用其他语言实现（例如 Python 或 Go），这块的内部工作机制和开销有何不同？
   - **解答**：Python 的列表和对象是通过引用计数的，底层常常需要经过散列表查找或创建全新的对象来替代（例如 `msg['content'] = "新字符串"` 实际上是将字典中键值指向了一个新创建的字符串对象，旧字符串会在没有引用后被垃圾回收）。Go 语言虽然也能用指针就地修改，但如果切片扩容或者涉及 interface 转换，仍然会有一定的 GC 开销。
而在 Rust 中，`*content = ...` 是一次精准的内存重写。原来那块堆内存的字符串分配被直接 Drop 掉（释放），并在原有的变量绑定处接上新的堆内存地址。这一切都是安全且在抛弃垃圾回收（GC）的前提下完成的，效率极高。

2. 为什么在 Trait 的 `compress()` 签名中，我们需要标注 `<'a>` 和 `'a` 这样的生命周期参数呢？
   - **解答**：因为我们要返回一个存放于堆上的异步代码块：`Pin<Box<dyn Future + 'a>>`。这个 Future（即代码块 `async move { ... }`）在随后被 `.await` 之前，需要持有对 `messages` 的借用。生命周期 `'a` 就是向编译器担保：**这个被返回在盒子里的异步任务，活得绝不会比它借用的 `messages` (或者 `client`) 更长**。否则，异步任务执行到一半，原来的 `history` 已经被销毁了，就会引发野指针和数据竞争。这就是 Rust 伟大的借用检查器。

---

## 避坑指南与 Debug 学问 (来自真实测试的血泪经验)

如果你在实战测试时遇到了奇怪的问题，大概率是下面这两个：

### 1. 臭名昭著的 UTF-8 字节越界 Panic
如果日志突然崩溃，控制台甩出：`byte index 80 is not a char boundary; it is inside '结' (bytes 79..82)`
- **原因**：在输出提示词日志时，我们为了省事写了截断：`&prompt[..prompt.len().min(80)]`。在 Python 中 `[0:80]` 只是切出 80 个字符；而在 Rust 中，字符串底层是 UTF-8 字节数组。因为汉字占用 3 字节，第 80 个**字节**可能刚好砍在半个中文字符上！出于对内存与字符编码绝对的安全性保障，Rust 直接触发 Panic。
- **解法**：告别粗暴的切片，请拥抱安全的字符迭代器：
  ```rust
  let safe_prompt: String = prompt.chars().take(80).collect();
  ```

### 2. 陷入无限 Auto Compact ()
如果控制台疯狂连环输出：`[Auto Compact触发] 上下文过长...` 且没有任何有效执行：
- **原因**：在测试组装 `SummaryCompressor` 的时候，可能图省事把 `max_len` 设为了极短的数值（比如 `1000`）。但调用了 LLM 后返回的“总结文档”本身就有可能有几千个字符长。当这个总结作为重置后的唯一请求，进入下一个回合时，立刻又超过 `1000` 的阈值，于是再度激活大模型进行缩写，活活逼死。
- **解法**：在能够跑通后，务必在 `main` 循环配置处，将 `SummaryCompressor` 的 `max_len` 修正为真实的生产阈值，例如 `50000` 起。

---

## 完成标准

- 成功引入 `Compressor` Trait 以及两个实现体。
- 将原本纯粹的 `agent_loop` 改为接受动态或静态的压缩器集合，并能够正确在循环中调用。
- 修复截断字体的崩溃问题与连环缩写阈值问题。
- 原有的功能（bash、读写等）不受到影响，且能够在日志看到 `[已压缩]` 的提示。
