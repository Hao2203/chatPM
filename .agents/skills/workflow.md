# Backend Development Workflow

## Three-Layer Architecture

后端开发按三层递进：**核心层 → 适配层 → 组装层**。每一层完成后方可进入下一层。

```
              ┌──────────────────┐
              │  3. 组装层        │  Tauri commands, main, 依赖注入
              │  (Assembly)      │  唯一可以引入所有 crate 的地方
              └────────┬─────────┘
                       │
        ┌──────────────┼──────────────┐
        │              │              │
  ┌─────┴─────┐  ┌─────┴─────┐   其他适配器
  │ Database   │  │ API Client│   (未来扩展)
  │ Adapter   │  │ Adapter   │
  └─────┬─────┘  └─────┬─────┘
        │              │
  ┌─────┴──────────────┴─────┐
  │  2. 适配层 (Adapters)     │  包含 I/O、网络、文件系统、数据库
  │  trait 定义 + 实现        │  依赖核心层，不互相依赖
  └────────────┬─────────────┘
               │
  ┌────────────┴─────────────┐
  │  1. 核心层 (Core)         │  纯数据类型 + 纯函数
  │  零副作用，零 I/O         │  零外部依赖（除 derive_more）
  └──────────────────────────┘
```

---

## Layer 1: Core Crate — 核心类型与纯函数

**目标：** 定义领域模型，所有计算均为纯函数，可独立编译、测试。

### 规则

| 允许 | 禁止 |
|------|------|
| `struct` / `enum` / `impl` | `async fn` |
| 纯变换函数（输入 → 输出） | `tokio` / 任何异步 runtime |
| `From` / `Display` / `Serialize` / `Deserialize` | 文件 I/O、网络、数据库 |
| `#[cfg(test)]` 中的单元测试 | 环境变量读取 |
| `derive_more` | 任何带副作用的 crate |

### 产出清单

- [ ] 核心类型定义（`Message`、`Role`、`Context`、`Turn`、业务专属 newtype）
- [ ] 纯函数实现（`compose_prompt()`、`normalize()`、`assemble()`、`compose()`）
- [ ] 状态类型定义（生命周期状态如 `NewSession`、`TitlePrompt`、`Session`）
- [ ] 不涉及 I/O 的验证逻辑（格式校验、字符过滤）
- [ ] 单元测试覆盖所有公开函数

### 核心层设计模式

#### Newtype Pattern

对所有外部标识符和值对象使用 newtype 封装，禁止裸 `String` / `Uuid` 出现在领域类型的公开字段中：

```rust
// ✅ 正确
pub struct SessionId(Uuid);
pub struct Title(String);

pub struct Session {
    pub session_id: SessionId,
    pub title: Title,
}

// ❌ 错误：裸类型
pub struct Session {
    pub session_id: Uuid,
    pub title: String,
}
```

每个 newtype 需实现：`Display`（用于日志）、适当访问器（`as_str()`、`as_uuid()` 等）。

#### Type-Driven State Machine

用 Rust 类型系统建模业务流程的状态转换，使非法状态不可表达：

```
NewSession ──into_title_prompt(self)──→ TitlePrompt ──finalize_session()──→ Session
                                                                              │
                                                                        chat(&Session)
```

关键规则：
1. **每个状态是一个类型** — 不含 `Option` 来判断"是否已完成某步骤"
2. **状态转换消耗前一个状态** — `fn transition(self, ...) -> NextState` 防止重复转换
3. **只有最终状态暴露业务操作** — `chat()` 只接受 `&Session`，不接受 `NewSession`
4. **提示词构造是纯函数** — `TitlePrompt::compose() → Vec<ChatMessage>` 在核心层完成

### 示例结构

```
crates/my_core/
├── Cargo.toml          # 仅依赖 derive_more
└── src/
    ├── lib.rs
    ├── types.rs         # 核心数据结构
    ├── transform.rs     # 纯变换函数
    └── tests.rs
```

---

## Layer 2: Adapter Crates — 副作用代码适配器

**目标：** 为核心层需要的副作用操作提供具体实现。每个适配器是一个独立 crate，只做一件事。

### 规则

| 允许 | 禁止 |
|------|------|
| 依赖核心层 crate | 依赖其他适配器 crate |
| `async fn`、tokio（仅在必要时） | 包含业务逻辑 |
| 数据库连接、HTTP 请求、文件操作 | 直接依赖组装层 |
| 暴露清晰的 trait / 公开 API | 跨适配器的隐式耦合 |

### 设计原则

1. **一个适配器一个职责：** 数据库 ORM → 一个 crate；API 客户端 → 另一个 crate；文件存储 → 另一个 crate。
2. **面向接口暴露：** 适配器的公开 API 最好是 trait，方便测试时 mock。
3. **可替换性：** 更换数据库或 API 服务商时，只需换一个适配器 crate。
4. **错误转换：** 将底层库的错误转换为领域错误类型（定义在核心层）。

### 产出清单

- [ ] 适配器 crate 骨架（`Cargo.toml` 依赖核心层 + 具体实现库）
- [ ] trait / 公开 API 定义
- [ ] 具体实现（如 `SqliteDb`、`DeepSeekClient`）
- [ ] 构造器 / 配置注入（连接串、API key 等作为参数传入，不读环境变量）
- [ ] 集成测试（使用真实或嵌入式实例，如 `open_in_memory()`）

### 示例结构

```
crates/my_api_client/
├── Cargo.toml          # my_core + reqwest + secrecy
└── src/
    ├── lib.rs
    ├── config.rs        # 连接配置
    ├── client.rs        # 具体实现
    └── tests.rs         # 集成测试（可能需要真实凭据）
```

---

## Layer 3: Assembly — 组装与编排

**目标：** 将核心层与适配器层连接在一起，实现完整的业务链路。

### 位置

组装代码放在最上层的 crate（例如 `chat_pm_service`）或应用入口（`src-tauri` / `main.rs`）。

### 职责

1. **依赖注入：** 创建适配器实例，注入到编排器中。
2. **编排流程：** 调用核心层纯函数 + 适配器的副作用操作，组成业务流程。
3. **配置管理：** 读取环境变量 / 配置文件，传递给适配器和核心层。
4. **错误处理：** 将所有子 crate 的错误统一处理，转换为面向用户的错误。

### 规则

| 允许 | 禁止 |
|------|------|
| 依赖所有核心 + 适配器 crate | 在适配器 crate 之间引入直接依赖 |
| 依赖注入、编排逻辑 | 将编排代码下放到适配器层 |
| 环境变量读取、tracing/logging | 核心层或适配器层读环境变量 |

### 产出清单

- [ ] 编排结构体（如 `ChatPipeline`），持有适配器实例
- [ ] 编排函数，串联核心层 + 适配器调用
- [ ] 配置聚合（从环境变量 / 文件加载，初始化所有组件）
- [ ] 集成测试（`demo` 风格的全链路测试）

---

## Development Order (Strict)

```
1. Core crate  ──→  编译通过 + 测试绿色
                          │
2. Adapter crate ──→  编译通过 + 测试绿色  (可并行开发多个适配器)
                          │
3. Assembly      ──→  全链路测试通过
```

**原则：** 前一层不稳定，不进入下一层。核心层变更会传导到适配层和组装层，适配层变更只影响组装层。

---

## Quick Checklist for New Features

当需要新增功能时，按以下顺序思考：

1. **这个功能的纯逻辑是什么？** → 放进核心层（类型 + 纯函数）
2. **它需要什么副作用？** → 新建或扩展现有适配器（I/O 操作）
3. **如何连接？** → 在组装层编写编排代码
