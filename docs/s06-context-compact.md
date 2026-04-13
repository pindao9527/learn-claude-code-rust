# s06: Context Compact (上下文压缩)

`s00 > s01 > s02 > s03 > s04 > s05 > [ s06 ] > s07 > s08 > s09 > s10 > s11 > s12 > s13 > s14 > s15 > s16 > s17 > s18 > s19`

> *上下文不是越多越好，而是要把“仍然有用的部分”留在活跃工作面里。*

## 这一章要解决什么问题

到了 `s05`，agent 已经会：

- 读写文件
- 规划步骤
- 派子 agent
- 按需加载 skill

也正因为它会做的事情更多了，上下文会越来越快膨胀：

- 读一个大文件，会塞进很多文本
- 跑一条长命令，会得到大段输出
- 多轮任务推进后，旧结果会越来越多

如果没有压缩机制，很快就会出现这些问题：

1. 模型注意力被旧结果淹没
2. API 请求越来越重，越来越贵
3. 最终直接撞上上下文上限，任务中断

所以这一章真正要解决的是：

**怎样在不丢掉主线连续性的前提下，把活跃上下文重新腾出空间。**

## 先解释几个名词

### 什么是上下文窗口

你可以把上下文窗口理解成：

> 模型这一轮真正能一起看到的输入容量。

它不是无限的。

### 什么是活跃上下文

并不是历史上出现过的所有内容，都必须一直留在窗口里。

活跃上下文更像：

> 当前这几轮继续工作时，最值得模型马上看到的那一部分。

### 什么是压缩

这里的压缩，不是 ZIP 压缩文件。

它的意思是：

> 用更短的表示方式，保留继续工作真正需要的信息。

例如：

- 大输出只保留预览，全文写到磁盘
- 很久以前的工具结果改成占位提示
- 整段长历史总结成一份摘要

## 最小心智模型

这一章建议你先记三层，不要一上来记八层十层：

```text
第 1 层：大结果不直接塞进上下文
  -> 写到磁盘，只留预览

第 2 层：旧结果不一直原样保留
  -> 替换成简短占位

第 3 层：整体历史太长时
  -> 生成一份连续性摘要
```

可以画成这样：

```text
tool output
   |
   +-- 太大 -----------------> 保存到磁盘 + 留预览
   |
   v
messages
   |
   +-- 太旧 -----------------> 替换成占位提示
   |
   v
if whole context still too large:
   |
   v
compact history -> summary
```

手动触发 `/compact` 或 `compact` 工具，本质上也是走第 3 层。

## 关键数据结构与抽象 (Rust 专属)

在 Rust 中，我们不推荐像 Python 那样使用全局的粗粒度函数。为了应对不同且多变的压缩策略（如单纯修建特定数据的微压缩、或是调用模型的总结压缩），我们引入**“特性（Trait）”和“动态分发”**概念：

```rust
use std::future::Future;
use std::pin::Pin;

pub trait Compressor {
    // 我们的压缩可能是“就地修改旧数据”，也可能要返回结果，所以传入 &mut Vec<Message> 可变借用
    fn compress<'a>(
        &'a self,
        messages: &'a mut Vec<Message>,
        client: &'a Client,
        model_id: &'a str
    ) -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>>;
}
```

通过这一层抽象，我们可以让 Agent 随时随地加挂多种不同的压缩策略组合。

## 最小实现

### 第一步：大工具结果先写磁盘（概念）

当工具输出太大时，不要把全文强塞进当前对话。最小标记可以长这样：

```rust
// 伪代码：在 run_bash() 等底层工具里直接判断：
if output.len() > PERSIST_THRESHOLD {
    let stored_path = save_to_disk(tool_use_id, &output);
    let preview = &output[..2000];
    return format!(
        "<persisted-output>\n\
         Full output saved to: {}\n\
         Preview:\n{}\n\
         </persisted-output>",
        stored_path.display(),
        preview
    );
}
```

### 第二步：旧工具结果做微压缩 (MicroCompressor)

对于旧工具，不需要保留大段的 `content`。因为拥有 `&mut Vec<Message>`，在 Rust 里可以**不分配新内存，就地进行修改**：

```rust
pub struct MicroCompressor {
    pub keep_recent: usize,
}

impl Compressor for MicroCompressor {
    fn compress<'a>(&'a self, messages: &'a mut Vec<Message>, _c: &'a Client, _m: &'a str) 
    -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>> {
        Box::pin(async move {
            let mut tool_count = 0;
            // rev()反向变量，先碰到的是最新的工具
            for msg in messages.iter_mut().rev() {
                if let Message::Tool { content, .. } = msg {
                    tool_count += 1;
                    if tool_count > self.keep_recent && content.len() > 120 {
                        // 利用借用，就地切除
                        *content = "[Earlier tool result compacted.]".to_string();
                    }
                }
            }
            Ok(false) // 只有微压缩，并没有触发全局清空
        })
    }
}
```

### 第三步：整体历史过长时，做一次完整总结 (SummaryCompressor)

当上面的手段都救不了疯狂膨胀的对话长度时，我们彻底重写历史：

```rust
pub struct SummaryCompressor {
    pub max_len: usize,
    pub api_key: String,
    pub base_url: String,
}

impl Compressor for SummaryCompressor {
    fn compress<'a>(&'a self, messages: &'a mut Vec<Message>, client: &'a Client, model_id: &'a str) 
    -> Pin<Box<dyn Future<Output = Result<bool, Box<dyn std::error::Error>>> + 'a>> {
        Box::pin(async move {
            let serialized = serde_json::to_string(messages).unwrap_or_default();
            if serialized.len() < self.max_len {
                return Ok(false);
            }

            // 发起异步的总结请求...
            let summary = fetch_summary(client, &serialized, model_id, &self.api_key, &self.base_url).await?;

            messages.clear(); // 暴力清空！
            messages.push(Message::User {
                content: format!("Conversation compacted.\n\n{}", summary)
            });

            Ok(true) // 上下文已被截断重写！
        })
    }
}
```

### 第四步：在主循环里接入动态分发的压缩数组

```rust
async fn agent_loop(
  client: &Client,
  // ... 其他参数
  messages: &mut Vec<Message>,
  compressors: &[Box<dyn Compressor>], // 接收一组通过 Box 打包的特质对象 (Trait Objects)
) -> Result<(), Box<dyn std::error::Error>> {
  loop {
    // 每次发送请求前，挨个过一遍过滤网
    for comp in compressors {
        let fully_compacted = comp.compress(messages, client, model_id).await?;
        if fully_compacted { break; } // 如果直接重写了历史，后面的就不用再看了
    }

    // 接下来再将 messages 序列化送给大模型进行正常的对话响应
    // ...
```

## 压缩后，真正要保住什么

这是这章最容易讲虚的地方。压缩不是“把历史缩短”这么简单。真正重要的是：**让模型还能继续接着干活。** 所以一份合格的压缩结果，至少要保住下面这些东西：

1. 当前任务目标
2. 已完成的关键动作
3. 已修改或重点查看过的文件
4. 关键决定与约束
5. 下一步应该做什么

如果这些没有保住，那压缩虽然腾出了空间，却打断了工作连续性。

## 它如何接到主循环里

从这一章开始，主循环多了一个很关键的责任：**管理活跃上下文的预算**。也就是说，agent loop 现在开始同时维护两件事：
`任务推进` 和 `上下文预算`。

这一步非常重要，因为后面的很多机制都会和它联动：

- `s09` memory 决定什么信息值得长期保存
- `s10` prompt pipeline 决定哪些块应该重新注入
- `s11` error recovery 会处理压缩不足时的恢复分支

## 初学者最容易犯的错

### 1. 以为压缩等于删除
不是。更准确地说，是把“不必常驻活跃上下文”的内容换一种表示。这也是为什么我们在 Rust 中用 `*content = ...` 就地替换而不是粗暴的 `Vec::remove`。

### 2. 压缩死循环陷阱（Debug 经典）
当你的测试阈值（如 `max_len = 1000`）配置得比大模型可能返回的 `Summary（总结）` 本身还要短时，第一回合截断完得到的“一句话历史”，立刻就会引发第二回合继续报错上限截断，形成无限递归缩水循环。必须根据实际生产模型（如 128k Token 上限）预留几万字符的真实空间。

### 3. 把压缩和 memory 混成一类
压缩解决的是：**当前会话太长了怎么办**
memory 解决的是：**哪些信息跨会话仍然值得保留**

## 一句话记住

**上下文压缩的核心，不是尽量少字，而是让模型在更短的活跃上下文里，仍然保住继续工作的连续性。而 Rust 为这套多维度的机制赋予了特质(Trait) 和就地借用(Borrow)的强力基建。**
