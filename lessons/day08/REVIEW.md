# Day 08 复习卡：并发调度与异步锁机制

## 1. 核心理论：多所有权与内部可变性

在 Python 等 GC 语言中，随便在哪个线程塞数据都不会报错。
但在 Rust 中，变量有且只有一个所有者。要让主循环读队列、后台任务写队列，必须动用 **共享状态并发模型**：

```rust
// Arc: 允许多个所有者 (Atomically Reference Counted)
// Mutex: 保证同一时刻只有一个人能修改内部数据
pub tasks: Arc<Mutex<HashMap<String, BgTask>>>,
```

## 2. 闭包捕获与 `move` 关键字

当我们把代码丢给后台时：

```rust
let manager_clone = self.clone();
tokio::task::spawn_blocking(move || {
    // manager_clone 的所有权被永久转移给了这个闭包
});
```

`move` 关键字强制闭包获取它使用到的变量的所有权，而不是仅仅借用。如果没有 `move`，编译器会抱怨闭包存活的时间可能比当前函数长，导致悬垂引用（Dangling Reference）。

## 3. `tokio::spawn` vs `tokio::task::spawn_blocking`

- **`tokio::spawn`**: 用于**异步**代码（等待 I/O，比如网络请求、等待另一个异步任务）。它不会阻塞调度器，几十万个 task 也能轻松在一两个 OS 线程中跑满。
- **`tokio::task::spawn_blocking`**: 用于**阻塞**代码（比如繁重的 CPU 计算、我们现有的 `std::process::Command::new` 或者是传统的文件 I/O）。Tokio 会为此专门开辟/分配后备的 OS 线程池，以免卡死处理异步任务的主力 worker 线程！

这就是为什么我们今天在包裹老的 `run_bash` 时，果断选择了 `spawn_blocking`。

## 4. `std::mem::take` 魔法：瞬间置空

我们在排空通知队列时，用到了极其优雅的一招：

```rust
let mut notifs = self.notifications.lock().unwrap();
std::mem::take(&mut *notifs)
```

`take` 会把数据整个抽走返回，并在原地留下一个该类型的**默认值**（对于 `Vec` 就是一个空的数组），而且**不用分配新的堆内存**。这比传统的 `.drain(..).collect()` 或者 `.clone()` 效率高太多！

## 5. 架构反思：不要什么能力都给子 Agent

我们之前发现了一个设计隐患：给子 Agent 下发后台命令的能力。
看似功能强大了，实则违背了 **“系统职能划分”** 的第一性原理：
1. 子 Agent 循环内部没有写 `drain_notifications()`，所以就算它发起了耗时任务，它也是个“聋子”，收不到结果。
2. 子 Agent 寿命短，活不到长耗时任务结束。
3. 它发出的任务如果在那时候结束，通知会突然跳到主 Agent 的上下文里，导致老大哥大模型产生严重的幻觉（Hallucination）。

**教训：代码不是能跑就行，权力和生命周期必须对齐！**
