# Day 01 复习卡：基础语法与控制流

## 1. 变量 (Variables)

- **不可变性 (Immutability)**: `let x = 5;` 之后不能 `x = 6;`。Rust 强制你思考哪些数据是该变的。
- **可变性 (Mutability)**: `let mut x = 5;` 允许修改。
- **隐藏 (Shadowing)**: 
    ```rust
    let x = 5;
    let x = x + 1; // 重新定义 x，可以改变类型
    let x = "hello"; 
    ```
  - **对比**: `shadowing` 创建了新变量，而 `mut` 只是修改了原变量的值。

## 2. 数据类型 (Data Types)

- **标量类型 (Scalar)**: `i32`, `u32`, `f64`, `bool`, `char` (注意：char 是 4 字节的 Unicode)。
- **复合类型 (Compound)**:
    - **元组 (Tuple)**: `let tup: (i32, f64, u8) = (500, 6.4, 1);`（长度固定）。
    - **数组 (Array)**: `let a = [1, 2, 3];`（长度固定，存储在栈上）。

## 3. 函数 (Functions)

- **参数名与类型**: 必须明确声明参数类型。
- **返回值**: 
    - 使用 `->` 声明。
    - **表达式 (Expression)**: 函数最后一行不加分号，其值即为返回值（常用）。
    - **语句 (Statement)**: 以分号结尾，不返回值（其实返回的是空元组 `()`）。

## 4. 控制流 (Control Flow)

- **if 表达式**: 
    ```rust
    let number = if condition { 5 } else { 6 }; // if 是有返回值的！
    ```
- **循环 (Loops)**:
    - `loop`: 无限循环。
    - `while`: 条件循环。
    - `for`: 遍历集合（最常用）。
        ```rust
        for element in [10, 20, 30].iter() { ... }
        ```

---
### 💡 第一天心得：
Rust 的语法初看像 C++，但它的 `if` 和大量的表达式特性让它写起来更像函数式语言。
