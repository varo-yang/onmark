# Onmark 架构设计

> 状态：目标架构初稿。本文区分不可动摇的系统原则、分阶段交付路径和后期生产能力，避免用终局蓝图指挥第一天施工。

本文与《Onmark 语言规格书》平级。语言规格负责“创作者如何表达影片”，本文负责“已编译的影片如何成为成片”，两者只通过 versioned Timeline IR 接合。

```text
Language Specification                     Render Architecture
screenplay → semantics → diagnostics       render graph → execution → artifacts
                    └──── Timeline IR ────┘
```

文中内容分为三个成熟度：

- **基础原则**：现在写进代码，并保持稳定；
- **交付关卡**：按顺序验证，上一关没有通过就不建设下一关；
- **生产终局**：方向已知，但只有在指标和真实负载出现后施工。

## 1. 系统定义

Onmark 是一个以剧本为源语言、以浏览器为画布、以确定性 Render IR 为执行合约的视频编译与渲染引擎。

它必须完整解决四件事：

1. 让人和 LLM 用接近剧本的结构描述视频；
2. 把内容、素材和少量显式时间关系编译成唯一时间线；
3. 把浏览器渲染变成可重放、可切片、可缓存的确定性任务；
4. 用同一套执行协议支持本地 CLI、单机服务和分布式 worker。

```text
Screenplay + Components + Assets
              │
              ▼
      Rust Compiler Core
              │
        deterministic IR
              │
              ▼
      Render Graph Planner
       ┌──────┼──────┐
       ▼      ▼      ▼
    Worker  Worker  Worker
       └──────┼──────┘
              ▼
        Assemble + Encode
              │
              ▼
             MP4
```

## 2. 六条架构公理

### 源语言表达意图，IR 表达事实

`<scene>`、`<shot>`、`<vo>` 和 cue 是创作意图；绝对帧区间、依赖边、缓存键和渲染分片是编译事实。两者不能混成一个万能 `Document`。

### 编译器是纯函数

相同文档、素材元数据、编译选项和版本必须产生 byte-identical IR。编译器不访问网络、不生成素材、不读墙钟，也不启动浏览器。媒体探测属于编译前 IO，其结果作为显式输入。

### 分布式不是另一套渲染器

CLI 与集群执行相同的 `ExecutionPlan` 和 worker 状态机。本地模式只是 coordinator 与 worker 同进程运行，不能有日后删除的“简化渲染路径”。

### 分片由像素依赖决定

`shot` 是优秀的创作和缓存候选边界，却不是无条件执行边界。转场、贯穿元素、全局层、shader history 和相邻采样都会产生跨镜头依赖。规划器必须先求依赖闭包，再切任务。

### 浏览器只负责画，不负责决定

Chromium 不求时间、不发现素材、不选择分片。它只接收已求解的帧号、场景状态和资源清单，在唯一主时钟下画出一帧。

### 每个昂贵结果都可寻址

素材探测、browser bundle、渲染单元、混合音轨和最终封装都有内容哈希。缓存正确性来自确定性，不来自文件名约定。

## 3. TS 与 Rust 怎么分

边界沿“浏览器世界”和“确定性系统世界”切开：

| 领域 | TypeScript | Rust |
| --- | --- | --- |
| 剧本 authoring 类型与组件 API | 主责 | 验证最终文档 |
| DOM/CSS/Canvas/WebGL/Three.js | 主责 | 不重写浏览器绘制 |
| 浏览器主时钟、seek、frame-ready | 主责 | 发命令并校验协议 |
| WAAPI/GSAP 等动画适配器 | 主责 | 消费能力声明 |
| TS/JS bundling | Node/esbuild 适配 | 生成 manifest、启动工具 |
| Parse、名称绑定、时间求解 | 不重复实现 | 主责 |
| Typed IR、Render Graph、分片 | 只消费协议 | 主责 |
| 缓存键、调度、幂等、重试 | 不负责 | 主责 |
| Chromium/FFmpeg 生命周期 | 浏览器内应答 | 主责 |
| CLI、worker、coordinator | 可提供 JS wrapper | 主责 |

Rust 编译器不是为了“HTML 解析更快”，而是因为它是系统信任根：phase type 固化 Parse → Structural Bind → Resolve → Solve → Lower，newtype 区分帧号、帧数和时间基，enum 穷尽时间规则与诊断，同一内核可直接嵌入 CLI、worker 和 coordinator。未来需要浏览器调用时，可以从同一内核构建 WASM/N-API binding，不能维护第二份求时逻辑。

## 4. 六种核心表示

```text
Source AST
  → Structurally Linked Film
    → Resolved Film
      → Timeline IR
        → Render Graph
          → Execution Plan
```

### Source AST

保留源码结构、属性原文和 span，用于精确诊断。允许未知标签、错误引用和未解析时间。

### Structurally Linked Film

元素词汇、合法包含关系与 film 全局 ID 已完成绑定；尚未解析的 authored attributes 仅作为 compiler 私有相位输入保留。

### Resolved Film

duration、cue、素材引用与内容起点已变成带 source span 的类型值，不再向公有 API 暴露 syntax-layer attributes。

### Timeline IR

所有时间规则已经求解成准确区间：

```rust
pub struct TimelineNode {
    pub id: NodeId,
    pub interval: FrameInterval,
    pub source: SourceSpan,
    pub reason: Vec<TimingReason>,
}
```

使用整数帧或有理时间基，禁止裸 `f64`。`reason` 解释“为什么在这里”，同时服务诊断、调试和增量失效。

### Render Graph

Timeline IR 回答“何时存在”；Render Graph 回答“这一帧依赖什么”。图中包括 DOM layer、素材帧、转场左右输入、persist 状态、滤镜历史窗口、字幕水印等全局层以及音频 clip。轨道只是这张图的只读投影。

### Execution Plan

这是 CLI、coordinator 和 worker 的稳定执行合约：

```rust
pub struct RenderUnit {
    pub id: RenderUnitId,
    pub output: FrameInterval,
    pub evaluation: FrameInterval,
    pub dependencies: Vec<ArtifactRef>,
    pub bundle: BundleRef,
    pub environment: RenderEnvironment,
    pub cache_key: ContentHash,
}
```

`output` 是最终提交的帧；`evaluation` 可以更宽，以覆盖转场预卷、弹簧动画 warm-up 和历史采样。worker 可以计算额外帧，但只发布 output。

## 5. 从源码到 MP4

### A. 装载并冻结输入

Loader 接收项目根、入口和渲染参数，解析本地引用并生成不可变输入清单。远程 URL 必须先下载进内容寻址素材库；编译和渲染不直接依赖会变化的 URL。

### B. 探测素材

Probe 使用 ffprobe 或原生解析器提取 duration、codec、尺寸、帧率、色彩信息和音轨布局，输出规范化 `AssetMetadata`，并按素材 hash 缓存。

### C. 编译

```text
parse → bind structure → resolve attributes/references → validate semantics → solve time → lower timeline
```

创作错误产生可聚合 diagnostics；机器故障返回 typed error。编译成功保证时间线唯一、自洽，但不意味着浏览器已经可执行。

结构 bind 与属性/引用 resolve 都会在构建候选产物的同时聚合创作诊断。只要存在 error，相位报告就不公开对应阶段值，避免被拒结构或恢复默认值被下一阶段误当成编译事实；warning 不阻塞产物。

Timeline solve 消费由 `onmark-core` 拥有的规范化 `AssetMetadata`；Gate 一首先只要求精确素材时长。`onmark-media` 通过探测生产这些 facts，ffprobe 专属结构与失败不得进入 core。引用的素材若未出现在调用方提供的 metadata map 中，属于 typed integration failure，而不是 authored diagnostic。媒体元素缺少 authored frozen artifact 时仍可通过静态 resolve，但无法产出可渲染 Timeline IR，并在 solve 阶段收到 authored asset diagnostic。

诊断是语言产品的一部分，不是日志。每条创作诊断必须包含稳定 code、源码 span、直接原因、相关节点，并在存在确定修法时给出可执行建议。建议面向人和 LLM 使用源码词汇，例如“定义 `cue:offer`，或将该标题改为相对当前 shot 的 `delay`”，不能只暴露求解器术语。

### D. 构建 browser bundle

Bundler 把用户组件、Onmark runtime、CSS 和静态依赖打成不可变 bundle。bundle 只包含绘制能力，不包含时间求解逻辑。manifest 记录 chunk、字体、外部素材、runtime 版本和能力声明，并进入缓存键。

### E. 构建 Render Graph 并分片

Planner：

1. 求每个输出帧的像素和音频依赖；
2. 把连续且依赖相近的帧合成候选区间；
3. 按转场、warm-up 和历史窗口扩展 evaluation interval；
4. 按成本、帧数、内存和失败域切成 Render Unit；
5. 计算稳定 cache key；
6. 分离视频捕获计划与音频计划。

普通顺序视频会自然按 shot/scene 切开；存在跨场景关系时，unit 会携带邻居依赖或自动合并，不会为了并行破坏画面。

### F. Worker 执行

```text
claim → materialize → launch → ready → seek/capture → encode → verify → commit
```

- `claim`：以租约领任务，重复领取仍安全；
- `materialize`：下载 bundle/依赖并校验 hash；
- `launch`：启动固定版本 Chromium 和编码器；
- `ready`：等待字体、图片、视频 decoder 和声明的异步资源稳定；
- `seek/capture`：Rust 发绝对帧号，runtime 设置时钟并返回 frame-ready；
- `encode`：帧经有界 pipe 进入 FFmpeg，编码速度反压捕获；
- `verify`：核对帧数、时间基、codec 和 hash；
- `commit`：临时写入后原子发布不可变产物。

### G. 音频和总装

音频不经过浏览器截图。Rust 从 Audio Plan 生成 FFmpeg filter graph 或 DSP 计划，完成裁剪、delay、fade、gain、重采样和混音。Assembler 验证所有视频段参数一致，优先 stream-copy 拼接，最后封装视频、音频和 metadata。

## 6. 确定性浏览器协议

唯一主时钟为：

```text
timestamp = frame_index / rational_timebase
```

禁止 `Date.now()`、真实 rAF 时间或自由运行的 media clock 决定成片。

协议至少包含：

```text
Load(plan_fragment)
Prepare(evaluation_start)
Seek(frame_index)
FrameReady(frame_index, state_hash)
Dispose
```

`FrameReady` 只能在 DOM update、layout、字体、图片 decode、视频 seek、WebGL submission 和框架 microtask 稳定后返回。超时要指出未稳定资源，不能只报 `page timeout`。

组件声明时间能力：

- `stateless`：任意帧直接 seek；
- `warmup(n)`：输出前需计算 n 帧；
- `sequential`：只能从 checkpoint 顺序推进；
- `global`：影响整个画面；
- `neighbor(radius)`：依赖前后时间窗。

Planner 根据声明选择分片。**未知组件默认 `sequential`，而不是 `stateless`**：可并行性必须被证明，不能被猜测。Onmark 原生动画可天然提供声明；官方 adapter 为 WAAPI、GSAP 等已验证用法提供声明；用户组件通过显式声明和确定性测试升级为 `stateless`。自动识别只提供建议，不能静默放宽正确性策略。

重复渲染检测是 conformance gate，但不能数学上证明任意用户代码无状态，因此不能作为危险默认值的补丁。

### 确定性分层承诺

“确定性”不能笼统等同于“最终 MP4 字节永远相同”：

| 层级 | 承诺 |
| --- | --- |
| Timeline IR、Execution Plan | 相同输入必须 byte-identical |
| 锁定 Chromium、字体、GPU/软件栈后的 raw frame | 目标为 frame hash 完全一致 |
| 跨异构机器的浏览器输出 | 以 conformance 结果定义支持范围，不提前承诺 |
| 编码后容器 | 校验时间戳、帧数、codec 和解码内容；是否 byte-identical 单独验证 |

缓存键必须匹配实际承诺的环境边界。不能为了 MP4 metadata 的字节顺序牺牲更重要的画面正确性。

## 7. 分布式模型（生产终局）

Coordinator 是控制面，只保存 DAG、租约、重试和产物引用，不转发帧。Worker 直接与对象存储交换 immutable bundle、素材和产物。

Worker 无状态，本地磁盘只是可丢缓存。队列保证至少一次执行；相同 cache key 可能重复计算，但 compare-and-commit 只发布一个不可变产物。

支持两种粒度：

- **Segment unit**：连续捕获并编码一段，默认模式；
- **Frame-range unit**：对昂贵且可随机 seek 的长场景继续切连续帧区间。

不把单帧做成远程任务。scheduler 按 CPU、内存、GPU、Chromium slot、encoder slot、codec、磁盘和网络能力匹配 worker。worker 内 browser 数、frame channel、下载并发和临时盘全部有界。

## 8. 缓存与修改底轨（基础模型，分阶段实现）

```text
Asset metadata
  → Typed IR
    → Browser bundle
      → Render unit artifact
        → Mixed audio
          → Final container
```

Render Unit 缓存键覆盖规范化 plan fragment、传递依赖 hash、compiler/runtime/Chromium/FFmpeg/font 环境、viewport、色彩、时间基、seed，以及 evaluation/output interval。

“修改底轨能否只重渲底轨”由依赖图决定：

- 上层透明且不采样底轨时，上层中间产物可复用；
- 浏览器一次合成所有层时，底轨变化会使重叠区间的最终帧失效；
- backdrop filter、blend mode、转场或 shader 读取底轨时，相关上层节点也失效；
- 可选择分层 alpha 中间产物换更细缓存，但会增加编码、颜色和合成成本。

Onmark 支持依赖驱动的增量渲染，但不承诺“每个 shot 永远独立”。正确性优先于缓存粒度。

## 9. 目标仓库边界

### 先模块，后 crate

领域概念不自动获得一个 crate。默认先放在现有 crate 的命名模块中；只有满足至少一条条件才拆包：

1. **运行环境不同**：浏览器、通用 native、Lambda handler；
2. **依赖预算不同**：纯编译内核不能被 Chromium/FFmpeg/AWS SDK 拖入；
3. **存在真实独立消费者**：有人只需要 compiler、runtime 或部署 SDK；
4. **部署或发布产物不同**：CLI binary、browser artifact、Lambda image 分别交付。

“代码很多”“名称听起来独立”“以后也许有用”都不是拆 crate 的理由。新增 crate/package 必须在 PR 中写明满足哪条标准、允许依赖谁、谁可以依赖它。

```text
onmark/
├── AGENTS.md  CLAUDE.md
├── README.md
├── Cargo.toml                 # Rust workspace
├── crates/
│   ├── core/                   # pure compiler + model + diagnostics + IR
│   ├── media/                  # 素材探测；不依赖 Chromium
│   ├── render/                 # browser/FFmpeg/executor，重型依赖边界
│   └── cli/                    # 人和 agent 的 native 入口
├── packages/
│   ├── runtime/                 # 浏览器主时钟、handshake、adapter modules
│   ├── authoring/               # TS 类型与组件 API
│   └── bundler/                 # Node/esbuild 与 bundle manifest
├── deploy/
│   └── aws-lambda/              # 第三关再加入：image、infra、示例
├── schemas/
├── conformance/
├── examples/
├── evals/
└── docs/
```

当前里程碑包含 `onmark-core` 与 `onmark-media`。Gate 一在对应行为首次被真实消费时再加入 `onmark-render` 与 `onmark-cli`：

- `onmark-core` 是纯内核，内部用 `syntax`、`diagnostics`、`model`、`compiler`、`timeline`、`protocol` 模块保持结构；
- `onmark-media` 只负责素材探测和规范化 metadata，使服务端 compile/lint 修正循环能够使用 `core + media` 而不链接 Chromium；
- `onmark-render` 是 Chromium、FFmpeg 编码和单机执行器的重型边界，它依赖 `core + media`；
- `onmark-cli` 只负责参数、终端展示和进程组装。

`onmark-media` 必须独立而不能藏在 render feature 中，因为“无 Chromium 的素材探测服务”是明确消费者，同时满足依赖预算和独立消费两条判据。Feature 只表达同一包内正交能力，不能用来遮住真实存在的架构边界。

Render Graph 和 planner 在第二关先作为 `onmark-core` 模块加入。只有出现独立消费者、编译成本或清晰发布边界后才考虑拆 crate。worker 状态机先属于 `onmark-render`；coordinator 是第三关的部署系统，不提前进入核心 workspace。

### Core 内部依赖也必须执法

合并成一个 crate 不等于允许模块互相穿墙。`onmark-core` 的内部 DAG 为：

```text
compiler ──→ syntax ──────→ model
    ├────→ diagnostics ───→ model
    ├────→ timeline ───────→ model
    └────→ model

protocol ─→ diagnostics / timeline / model
```

箭头表示“左侧可以依赖右侧”；精确允许边如下：

```text
model       → (none)
syntax      → model
diagnostics → model
timeline    → model
compiler    → syntax + diagnostics + timeline + model
protocol    → diagnostics + timeline + model
```

`syntax` 不得依赖 compiler，`timeline` 不得依赖 syntax，领域模块不得反向依赖 protocol。CI 使用 `syn` 对显式 Rust path 做语法感知检查。这是一条协作式护栏，覆盖普通路径、import、alias 和 re-export，但不覆盖宏内部生成的路径，也不等价于 rustc 的完整名字解析；这些边仍由评审负责。任何新增内部边必须先更新本文。

`onmark-core` 只允许 `syntax` 使用 `xmlparser` 做纯计算、保留 span 的 XML-compatible fragment tokenization。树构建、嵌套检查、重复属性检查、引用解码和全部创作语义由 Onmark 自己拥有；parser error 在 syntax 边界翻译，该依赖不执行 IO。测试 target 可以使用 `proptest` 验证时间代数，并使用 `syn` 执行协作式模块依赖律检查；二者都不会链接进库消费者或运行时产物。

`onmark-media` 只依赖 core，以及用于私有 ffprobe response 边界的 `serde`/`serde_json`。它使用参数数组直接启动配置的 ffprobe executable，绝不经过 shell；退出后仍让派生进程持有输出 pipe 的 wrapper 不属于该 executable contract。在这条 direct-child 契约下，进程寿命和保留的 stdout/stderr 字节数都有显式上限，两条 pipe 并发排空；显式 shutdown 会报告 process-control failure，`Drop` 只作 best-effort termination fallback。私有 ffprobe response type 只在此边界翻译一次并产出 core-owned `AssetMetadata`；JSON value 与第三方 error type 不定义稳定 API，但底层 error 会通过标准 source chain 保留，供调试使用。

校验失败原因保留为局部领域值。syntax 提供源码上下文后，由 `compiler` 模块唯一负责把 `InvalidNodeId` 等原因翻译成带源码位置的 `Diagnostic`，包括各阶段特有的 message 和 help；`diagnostics` 只拥有通用诊断表示与稳定 code。`model` 和 `syntax` 都不依赖 diagnostics，调用方也不得重复实现这层翻译。

### TypeScript package 方向

```text
@onmark/runtime  ←  @onmark/authoring
       ↑
       └──────────  @onmark/bundler
```

`runtime` 是浏览器底座和长期稳定扩展点，拥有当前帧 hook、FrameReady 协议、`stateless/warmup/sequential` 能力声明以及 adapter contract。`authoring` 只通过 runtime 的 types-only entrypoint 使用这些公开类型，不能依赖 runtime 的副作用入口。`bundler` 注入固定 runtime artifact 并生成 manifest；runtime 永不依赖 authoring 或 bundler。

### AWS Lambda 是适配器，不是第二套引擎

第三关引入 `@onmark/aws-lambda` 或等价独立部署包，因为它同时满足“独立消费者”和“独立部署产物”。它包含：

- invoke/result 的公开 SDK；
- SAM/CDK/Terraform 之一的基础设施入口；
- Lambda handler；
- 固定 Chromium、FFmpeg、字体和 Rust binary 的 container image；
- S3、任务分发和产物发布适配。

handler 只做矩形编排：

```text
decode invocation
→ materialize Render Unit
→ onmark-render::execute_unit
→ upload immutable artifact
→ return structured result
```

它不复制 compiler、frame handshake、FFmpeg plan 或 cache-key 逻辑。AWS 类型不允许进入 `onmark-core`；`onmark-render` 通过普通 filesystem/artifact-store 能力运行，不知道自己是否在 Lambda。Lambda container 使用只读镜像和 `/tmp` 临时工作区，worker 进程也使用同一个 image contract。

如果将来出现 ECS/Kubernetes backend，它们只是同一执行器的另一个 deploy adapter，而不是新 renderer。

### Schema 的单向来源

需要区分两类 TypeScript 类型：

- Timeline IR、Execution Plan、runtime message 属于跨进程 wire protocol；
- components、props、hooks 属于手写的 authoring API。

Rust wire types 是 source of truth。protocol 开始实现后，`cargo xtask schema` 从它们生成 versioned JSON Schema 和 TypeScript types/codecs，CI 重新生成并要求工作树零 diff。生成结果提交进仓库，供 npm package、diff review 和非 Rust 消费者直接使用；禁止手工修改。schema version 变化必须带 migration/conformance fixture。Rust 本身直接使用原始领域/wire types，不再从 schema 反向生成第二套 Rust 类型。

authoring API 可以追求浏览器端人体工程学，但不能复制求时语义。

```text
Rust wire types → checked-in versioned schema → generated TypeScript codecs

handwritten TypeScript authoring API → screenplay source → Rust compiler
```

## 10. 产品表面与可观测性

```text
onmark check film.html
onmark compile film.html --emit plan.json
onmark inspect plan.json --timeline
onmark render film.html -o film.mp4
onmark worker --coordinator ...
```

Rust API 用于嵌入服务端；TS API 用于 authoring；跨进程使用 versioned schema，不直接暴露内部领域对象。CLI 输出、诊断码和 Execution Plan 都是稳定产品协议。

每次 render 有 render ID，每个 unit 有 attempt ID。Trace 贯穿 compile、bundle、schedule、prepare、capture、encode、upload 和 assemble。核心指标包括单帧 capture/encode 时间、CPU/RSS、channel 深度、缓存命中、重试阶段、网络字节、临时盘峰值和 planner 估算误差。

## 11. 安全边界

用户 HTML/JS 是不可信代码。生产 worker 运行在隔离容器或 microVM：无宿主凭据、默认断网、只读 bundle、限定素材目录，并限制 CPU、内存、PID、磁盘和时间。

不能因为容器启动困难就关闭 Chromium sandbox。FFmpeg 参数使用数组而非 shell。远程素材下载处于独立 fetch 边界，限制 URL、重定向、大小和类型。

## 12. 三个交付关卡

### 第一关：稳定渲出一条真视频

唯一目标是证明核心闭环：

```text
Screenplay → Timeline IR → Browser Runtime → Chromium → FFmpeg → MP4
```

范围只有：最小剧本语言、素材探测、Rust 时间求解、versioned Timeline IR、TS 确定性时钟、FrameReady handshake，以及单进程单 Render Unit 的真实视频。必须测清字体、图片、视频 seek、异步稳定和捕获方式。

这一关不建设 coordinator、lease、远程 worker、能力调度和分层缓存。

### 第二关：正确地切开

把同一视频编译成两个 Render Unit，在本地独立捕获、编码并总装。此时实现 Render Graph、`evaluation/output` 区间、转场预卷、unit cache 和依赖闭包失效。修改一个 shot 后，只允许被图证明安全的产物复用。

### 第三关：离开本机仍然成立

让第二关的同一 Execution Plan 在两个独立 worker 进程执行，引入对象存储、lease、重试、幂等发布和能力调度。验收 IR/plan byte-identical、锁定环境下 raw frame hash 一致，以及解码后音视频等价；不预设 MP4 容器字节必然一致。

每一关都使用最终方向的 IR 和协议，但只实现本关真实消费的部分。上一关没有稳定通过，不创建下一关的空架子。

## 13. 待实验决策

- Chromium 控制选择 CDP、WebDriver BiDi 还是极薄现有库；
- 捕获选择 BeginFrame、screenshot、surface copy 还是编码流；
- 分层 alpha 缓存何时值得额外成本；
- Execution Plan 公开编码使用 JSON、Protobuf 或分层组合；
- coordinator 首版存储使用 SQLite/Postgres 还是可替换 trait；
- 哪些动画适配器可随机 seek，哪些必须 warm-up/sequential；
- 浏览器、字体与 FFmpeg 环境锁定到什么粒度。

实验优先级依次为：捕获方式与 FrameReady 正确性、未知组件的保守执行成本、分片与预卷、跨 worker 一致性、分层缓存收益。纯编译内核、确定性协议、依赖驱动分片和本地/分布式同构是基础骨架，不应反复摇摆。
