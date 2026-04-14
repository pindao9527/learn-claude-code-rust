# Day 07：权限系统 (Permission System) 与 异步编程深入 (Async/Await & Tokio)

对应原版：`s07_permission_system.py`

## 今天做什么

当我们的 Agent 具备了调用系统级工具（比如执行终端命令、修改本地文件）的能力后，**安全性（Safety）** 便成为了头等大事。我们绝不能允许大语言模型毫无顾忌地随意执行例如 `rm -rf /` 或者提权操作。

在原版的 Python 案例中，作者引入了一个基于管道（Pipeline）的权限系统。整个校验流为：
`拦截黑名单 (Deny Rules) -> 代理模式检测 (Mode Check: default/plan/auto) -> 允许白名单 (Allow Rules) -> 询问用户 (Ask User)`。

由于这是一个有序的异步流程（例如“询问用户”可能需要挂起等待来自终端的 `async` IO 输入），这为我们在 Rust 中深入学习 **异步并发编程 (Async/Await)** 与 **Tokio 调度机制** 提供了绝佳的实战场地！

工作流程：
1. 复制一份 `s06` 的基础代码作为起步点，或者在工作区里建好当天的脚手架。
2. 定义包含 `Deny`, `Allow`, `Ask`, `Plan`, `Auto` 概念的规则结构与枚举。
3. 实现基础的安全拦截器（比如基于正则表达式拦截 bash 命令）。
4. 编写 `PermissionManager` 的四阶段流水线（Pipeline），并在其中练习处理异步的终端挂起等待任务（比如使用 Tokio 读取终端输入）。
5. 融入到 `agent_loop` 中，确保所有的 `tool_use` 必须要经过权限系统的绿灯才能真正执行 `handler` 获取输出。

---

## 学习目标

1. 掌握 Rust 真正异步的心脏：深入理解 `Future` 以及它的底层调度（Executor/Poll）。
2. 在 Tokio 中使用异步 IO（如 `tokio::io::stdin` 等）来无阻塞地处理用户在命令行输入 `y/n/always` 授权指令。
3. 把上一步学到的 Trait 推广：把每一个四阶段拦截都理解成实现了相同 trait 的 middleware 链式调用（可选高级做法），体会“异步链”的优雅。
4. 加深对于 Rust 模式匹配处理状态枚举（Mode）的熟练度。

---

## 核心概念：异步管道与 Tokio

### Pipeline 的本质
在 Python 中，通常只是一些 `if...elif...` 同步阻塞代码。一旦遇到 `input("Allow?")`，整个线程就死死了。
在 Rust/Tokio 中，`await` 意味着如果不满足继续执行的条件，当前的任务会**主动让出（Yield）**CPU，系统可以转而调度别的任务（尽管目前我们还是单线程的等待，但理解这种非阻塞的并发哲学对后续 Day13~Day17 的自治多代理至关重要）。

### Error vs Denied
权限拒绝（Denied）并不代表系统出错（Panic 或 Error），而是一个正常的业务状态。在 Rust 中，我们要妥善将 `PermissionResult::Denied("危险指令")` 与系统的 `anyhow::Error` 分开处理。让大模型明确看到："Permission denied: 触发防暴走规则" 的结果并让模型进行自我纠偏。

---

## 思考题（写完后回答）

1. 在 Python 的源码里，安全校验就是写了几个正则表达式，而在 Rust 中引入外部 `regex` Crate 会有一定的编译时间开销，或者可以选择手写 `contains`。在这个特定场景下，你认为正则表达式还是精准字符串匹配在 Rust 中更安全或更高效？
2. 在异步的 `ask_user` 函数中，为什么要等待终端的 `.await` 输入而不能使用普通的 `std::io::stdin().read_line()`？如果用了普通的同步 IO 读取会发生什么“可怕”的事情？

---

## 完成标准

- 定义出 `BashSecurityValidator` 以及黑名单规则，自动在 `agent_loop` 被调用时拦截诸如 `sudo` 或 `rm -r` 的危险指令。
- 引入三态工作模式（Plan只读 / Auto部分自动 / Default手动核准）并在 Agent 运行时能根据指令切换模式。
- 实现交互式的 `ask_user`，当遇到未知权限的工具调用时挂起并等待用户异步输入，用户如果输入 `always` 能被更新到动态内存的 `Allow` 列表中。
- 代码成功编译并拒绝让 Agent 搞砸我们的电脑！
