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

`shot` 是优秀的创作和缓存候选边界，却不是无条件执行边界。Gate 二的第一版 graph 仅在当前生产 adapter 已证明不会跨 shot 保留状态的前提下，把每个 shot 记录为独立 region，并记录该 region 的直接冻结媒体依赖；这不是“shot 天然可切”的通用规则。转场、贯穿元素、全局层、shader history 和相邻采样都会产生跨镜头依赖，必须先扩大或合并 region，再由规划器求依赖闭包、切任务。

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

Rust 编译器不是为了“HTML 解析更快”，而是因为它是系统信任根：phase type 固化 Parse → Structural Bind → Resolve → Solve，newtype 区分帧号、帧数和时间基，enum 穷尽时间规则与诊断，同一内核可直接嵌入 CLI、worker 和 coordinator。验证发生在第一次拥有足够信息的相位；Solve 直接构造 Timeline IR。没有新表示去证明新不变量时，不添加仪式性的 Validate/Lower 相位。未来需要浏览器调用时，可以从同一内核构建 WASM/N-API binding，不能维护第二份求时逻辑。

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
pub struct TimelineTiming {
    interval: FrameInterval,
    start_reason: TimingReason,
    end_reason: TimingReason,
}
```

每个 Timeline 元素保留这份 timing fact。使用整数帧或有理时间基，禁止裸 `f64`。
区间的两个端点各自保留“为什么在这里”的原因，服务诊断、调试和增量失效；这些是
compiler fact，不会把 `start`、`end` 或 `begin` 属性带回 screenplay 语言。

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

编译管线在 Timeline IR 结束，执行管线从一条独立的组合边界开始：

```text
Timeline IR + Frozen Asset Catalog + Bundle Manifest + Render Profile
  → Render Unit
    → Browser Plan + Audio Plan + materialization requirements
```

这条接缝不是另一个编译相位。Timeline IR 只回答影片中什么事实在何时成立；presentation bundle 负责把这些事实画成 DOM、CSS、Canvas 或 WebGL；Render Unit 则定义一次 executor 调用消费哪些不可变输入。第一关只有一个覆盖整部影片的 unit。第二关加入 Render Graph 后，可以产生多个同类型 unit，但不更换执行器合约。

Gate 一的 `AudioPlan` 只包含已求解的旁白 placement。materialization 会把其冻结字节与浏览器素材一起复制，却不把它们变成浏览器输入。Chromium 编出视觉流后，executor 将每个音频输入归零、施加精确有理的帧延迟、混合轨道，并将 AAC mux 进最终 MP4。gain、fade、重采样策略和通用音频效果仍然延期，不能从这份首个混音合约中推断出来。一条 Gate 一 unit 最多保留 512 条音轨，使 `FFmpeg` 参数和 filter graph 边界始终有界。

Gate 二的首条本地组装路径会在各自独立 materialize 的 unit 依次把连续 output 帧送入同一个视觉 encoder 期间保留这些 unit root。旁白先保留绝对 Timeline 起点，最终总装时只按成片 output 原点重基一次，并在所有 unit 的画面都捕获完后混合。这样既不假定已 mux AAC 的分段可以安全拼接，也不再做一次有损的视频重编码。这是一条正确性优先的路径，不是持久分段缓存格式；缓存编码分段必须先有独立的等价性证明，才能成为执行产物。

## 5. 从源码到 MP4

### A. 装载并冻结输入

Loader 接收项目根、入口和渲染参数，解析本地引用并生成不可变输入清单。远程 URL 必须先下载进内容寻址素材库；编译和渲染不直接依赖会变化的 URL。

素材在三层身份之间显式转换，不能混用：

- `AssetRef` 是剧本中作者写下的逻辑引用；
- `FrozenAssetId` 标识实际被探测、被编译的不可变字节；
- materialized asset 是 worker 为同一份字节准备的本地路径或 browser URL。

Loader 或 composition root 先计算并验证 `FrozenAssetId`，probe 读取同一份已冻结字节并产生 `AssetMetadata`。Compiler 接收 `AssetRef → (FrozenAssetId, AssetMetadata)` catalog，Timeline IR 只保存 `FrozenAssetId`，绝不保存可变路径，也不把作者拼写误称为冻结身份。执行前的 materialize 再把冻结身份解析成 worker-local location，并复核 digest。

第一关的 `FrozenAssetId` 固定使用 SHA-256，canonical spelling 为 `sha256:<lowercase-hex>`。hash 计算属于 IO freezing boundary；core 只拥有已计算的身份与确定性映射，不读取文件。

### B. 探测素材

Probe 使用 ffprobe 或原生解析器提取 duration、codec、尺寸、帧率、色彩信息和音轨布局，输出规范化 `AssetMetadata`，并按素材 hash 缓存。

### C. 编译

```text
parse → bind structure → resolve attributes/references → solve Timeline IR
```

创作错误产生可聚合 diagnostics；机器故障返回 typed error。编译成功保证时间线唯一、自洽，但不意味着浏览器已经可执行。

结构 bind 与属性/引用 resolve 都会在构建候选产物的同时聚合创作诊断。只要存在 error，相位报告就不公开对应阶段值，避免被拒结构或恢复默认值被下一阶段误当成编译事实；warning 不阻塞产物。

Timeline solve 消费由 `onmark-core` 拥有的 `AssetRef → FrozenAsset` catalog；其中 `FrozenAsset` 绑定不可变身份与同一字节产生的规范化 `AssetMetadata`。`AssetRef` 是 screenplay-relative portable path，只允许 `/` 分隔，不能是绝对路径，不能含 `..`、空组件、`.`、反斜杠或平台前缀。metadata 记录精确素材时长，以及选中的音频和视觉流各自的精确 stream duration；视觉流还会记录 codec、pixel format，以及一个精确有理帧率或 variable timing。单帧流会单独建模，因为确切的单帧计数无法证明 source rate。`onmark-media` 通过探测生产 metadata，loader 或 composition root 负责冻结同一份字节；ffprobe 专属结构、路径与失败不得进入 core。引用素材若不在 catalog 中，属于 typed integration failure，而不是 authored diagnostic。媒体元素缺少 authored source 时仍可通过静态 resolve，但无法产出可渲染 Timeline IR，并在 solve 阶段收到 authored asset diagnostic。

诊断是语言产品的一部分，不是日志。每条创作诊断必须包含稳定 code、源码 span、直接原因、相关节点，并在存在确定修法时给出可执行建议。建议面向人和 LLM 使用源码词汇，例如“定义 `cue:offer`，或将该标题改为相对当前 shot 的 `delay`”，不能只暴露求解器术语。

### D. 构建 browser bundle

Bundler 把用户组件、Onmark runtime、CSS 和静态依赖打成不可变 bundle。bundle 只包含绘制能力，不包含时间求解逻辑。目标 manifest 会记录 chunk、字体、外部素材、runtime 版本和能力声明，并进入缓存键。Gate 一当前 manifest 只记录固定 entry document 与实际保留文件；这些文件的 hash 已经绑定注入的 runtime 与编译后 CSS。`bundleId` 是紧凑 UTF-8 JSON identity `{version,entryPoint,files}` 的 SHA-256；file 按 portable path 排序，每个 identity entry 的字段顺序固定为 `{bytes,path,sha256}`。这是 versioned contract，不是 pretty-printed manifest 的偶然表现。V1 包含一到 99,999 个 payload file；path 只能使用小写 portable ASCII，最长 1,024 bytes，不能进入 unit-owned namespace，也不能让一个 file 成为另一个 file 的目录祖先。其余字段只在 authoring 或 execution 真正消费时加入。

Presentation entry 拥有 DOM 结构、CSS/layout 与 runtime adapter 的安装；runtime 只提供确定性时钟、readiness 与媒体原语。Rust 不根据 Timeline IR 偷偷生成一套默认全屏 DOM。作者侧浏览器代码的公开规则写在 [presentation contract](presentation-contract.md)。

Gate 一组装一个 content-addressed unit root：所需素材位于 presentation entry 下的 `assets/sha256/<lowercase digest>`。browser 直接从 `BrowserPlan` 已携带的 frozen identity 推导这个相对位置，因此不需要第二套 native-path/browser-URL wire protocol。unit 只在 assembly 前保留 worker-local source path；materializer 复核精确字节后复制进私有 root，不用 link 把后续 source-path 变化带入执行。`RenderProfile` 拥有 viewport dimension 等会改变 pixel 的事实；process deadline 与 retained-memory ceiling 仍是 executor limit。materialization 会消费 `RenderUnit` 并产出同时拥有 `BrowserPlan` 与已验证私有 root 的 `ExecutableUnit`，executor 因而不可能把 plan 与无关 URL 或 asset root 拼在一起。

第一关不提前实现 Render Graph。它直接把整部 Timeline IR、冻结素材 catalog、bundle manifest 与 render profile 组合成一个 whole-film Render Unit：

```text
freeze inputs ─┬→ probe ─→ compile ───────────────┐
               └→ bundle presentation ───────────┤
                                                  ▼
                              whole-film Render Unit
                                → materialize Executable Unit
                                  → capture/encode → audio/mux → verify
```

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
materialize → launch → ready → seek/capture → fingerprint → verify → commit
```

- `materialize`：下载 bundle/依赖并校验 hash；
- `launch`：启动固定版本 Chromium；
- `ready`：等待字体、图片、视频 decoder 和声明的异步资源稳定；
- `seek/capture`：Rust 发绝对帧号，runtime 设置时钟并返回 frame-ready；
- `fingerprint`：把 capture PNG decode 成 canonical RGBA，并记录每帧 hash；
- `verify`：核对帧数、timebase、artifact payload 与 checksum；
- `commit`：临时写入后原子发布不可变 frame artifact。

capture worker 不拥有 visual encoder；未来 coordinator 才拥有 claim/lease，assembler
才拥有一条连续的 visual encode。

### G. 音频和总装

音频不经过浏览器截图。Rust 从 Audio Plan 生成 FFmpeg filter graph 或 DSP 计划，完成裁剪、delay、fade、gain、重采样和混音。Assembler 验证每份 frame artifact 的 unit identity 和 capture-environment identity，再把已验证 PNG 帧流送进一条连续 visual encoder，最后在 assembled output origin 一次性混音并发布。独立编码的视频段不假定可安全 stream-copy 拼接。

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
FrameReady(frame_index)
Dispose
```

`FrameReady(frame)` 是逻辑稳定屏障，只能在 DOM update、layout、字体、图片 decode、视频 seek、WebGL submission 和框架 microtask 稳定后返回。native executor 收到它后，会在既有 capture deadline 内等待两个 animation-frame turn，让 Chromium 将已选状态提交到 compositor，再捕获画面。direct rendering 把这个 PNG 留作 encoder payload；worker capture 额外把它 decode 成配置 profile 的精确 8-bit RGBA viewport，并对 canonical pixel bytes 做 hash。worker artifact 会把这个 raw-pixel hash 和每条有序 PNG record 一起记录，因此可比较独立 capture，而不把 PNG compression bytes 当作 visual truth。这个提交屏障不选择帧，也不成为时钟，只关闭 logical runtime 到 native capture 之间的竞态。runtime 不发布另一份自行定义的 state hash。runtime 内部的 `RuntimeFrame` 保留精确整数帧身份，只在调用浏览器 API 时从 Rust 给出的有理帧率推导浮点秒数；这个秒数永远不成为调度或协议事实。超时要指出未稳定资源，不能只报 `page timeout`。

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
| Timeline IR、Execution Plan | 存在 canonical encoding 后，相同输入必须 byte-identical；当前内存 IR 只承诺结构确定性 |
| 锁定 Chromium、字体、GPU/软件栈后的 raw frame | 目标为 frame hash 完全一致；worker artifact 的逐帧 fingerprint 将其变成可执行的一致性契约 |
| 跨异构机器的浏览器输出 | 以 conformance 结果定义支持范围，不提前承诺 |
| 编码后容器 | 校验时间戳、帧数、codec 和解码内容；是否 byte-identical 单独验证 |

缓存键必须匹配实际承诺的环境边界。不能为了 MP4 metadata 的字节顺序牺牲更重要的画面正确性。

## 7. 分布式模型（生产终局）

Coordinator 是控制面，只保存 DAG、租约、重试和产物引用，不转发帧。Worker 直接与对象存储交换 immutable bundle、素材和产物。

Worker 无状态，本地磁盘只是可丢缓存。队列保证至少一次执行；相同 cache key 可能重复计算，但 compare-and-commit 只发布一个不可变产物。

Gate 三先采用一个刻意收窄的 interchange：worker 把一个完整计划输出区间捕获为一份有界、带校验和的 frame artifact。它是单个版本化文件，记录精确 output interval、render profile、visual-plan 与 locked capture-environment identity，并携带有序 PNG 流及其 canonical raw-RGBA fingerprint。worker 在同目录 staging file 中写完后通过 no-clobber link 发布；重试只能验证或复用同时匹配计划 unit 与 capture environment 的已有不可变结果，永远不会读到半成品。assembler 会验证每份 artifact 对应其计划 unit 和 capture environment，再像 Gate 二一样把已验证帧流送进同一个连续 visual encoder，最后在 assembled output origin 一次性 materialize 并 mix 全部 audio。

这不是 remote-frame queue：一个 worker 独占连续 unit，只有 browser session 完成后才发布一个 artifact。它也不是 encoded-segment cache：不能假定独立 AAC-muxed MP4 可以安全 concat；独立 visual encode 也必须先有单独的等价性证明，才能替换无损 frame interchange。昂贵且已证明可 random seek 的长场景以后可继续切成连续 frame range。绝不把单帧做成远程任务。scheduler 按 CPU、内存、GPU、Chromium slot、encoder slot、codec、磁盘和网络能力匹配 worker。worker 内 browser 数、frame channel、下载并发和临时盘全部有界。

第一份实现只用 local filesystem 来证明 process 与 artifact contract，不引入 cloud SDK、queue、database、scheduler 或 deployment adapter。object storage、lease、retry ownership、capability matching，以及 Lambda/ECS/Kubernetes adapter，都必须等 artifact conformance 证明共同 executor boundary 正确后再加入。

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
├── scripts/                      # 仓库专用生成与质量检查
├── deploy/
│   └── aws-lambda/              # artifact conformance 后才加入：image、infra、示例
├── schemas/
├── conformance/
├── examples/
├── evals/
└── docs/
```

已完成的 Gate 一里程碑包含 `onmark-core`、`onmark-media`、`onmark-render`、`@onmark/runtime` 的浏览器 session、`@onmark/authoring` 的语义 DOM bindings、`@onmark/bundler` 的 presentation artifact 边界，以及第一条 `onmark-cli` whole-film composition root：

- `onmark-core` 是纯内核，内部用 `syntax`、`diagnostics`、`model`、`compiler`、`timeline`、`protocol` 模块保持结构；
- `onmark-media` 只负责素材探测和规范化 metadata，使服务端 compile/lint 修正循环能够使用 `core + media` 而不链接 Chromium；
- `@onmark/runtime` 因为运行在浏览器中、并被 authoring 与 bundler 消费而保持独立 package；
- `@onmark/authoring` 因为用户 presentation 会独立消费它的公开 DOM contract、而 runtime 不得向上依赖作者侧 effect 而保持独立 browser package；它唯一的产品依赖是 runtime 的 types-only 公开面；
- `@onmark/bundler` 因为运行在 Node、独占 esbuild 与文件系统依赖预算、并产出供 native renderer 独立消费的 presentation directory 而保持独立 package；
- `onmark-render` 是 Chromium、FFmpeg 编码和单机执行器的重型边界，只依赖 core-owned execution facts 与 render-owned materialized locations；
- `onmark-cli` 是独立发布产物，只负责参数、终端展示，以及 core compile、media probe、bundler process 和 native render 的组装，不把它们的实现揉进一个 crate。

Gate 一的 native 命令刻意保持很窄：`onmark render <screenplay>`。若未传 `--presentation`，它发现 screenplay 同目录的 `presentation.ts`；若未传 `--output`，它使用稳定且 no-clobber 的 `renders/<screenplay-stem>.mp4`。普通 render control 只有精确帧率和 viewport dimension，process path 只是 execution override，不是 screenplay fact。作者诊断先于 executable preflight 输出，因此解释一份无效剧本不要求机器先装好 Chromium、Node 或 FFmpeg。Gate 三新增刻意独立的 worker entry point：`onmark worker capture`。它只接受一份 versioned `request.json`（包含 deployment-owned、以 SHA-256 表示的 locked capture-environment identity）、该 manifest 列出的 `bundle/` payload 文件和冻结的 `assets/sha256/` 字节。这个 identity 覆盖 image 中的 Chromium、字体、launch configuration 及其他影响像素的 host facts；renderer 刻意不从单一 executable path 或 browser-version string 猜一个不完整的身份。worker 在私有 root 中 materialize 后发布一份 frame artifact，reuse 与 assembly 都要求 environment identity 和 unit identity 同时匹配。它不接受 screenplay、绝不重新编译 source；coordinator 或 object-store adapter 以后才负责发布 request。

`onmark-cli` 在启动外部工作前一次性解析全部 executable，然后按线性路径执行：read/compile → freeze/probe referenced assets → solve Timeline IR → bundle presentation → compose/materialize whole-film unit → render → atomic publish。冻结过程一边把每个引用源流式复制进私有临时文件一边计算 SHA-256，之后只 probe 这份私有副本，因此 identity 与 metadata 对应同一份 retained bytes。hash/probe 在显式 blocking work 中执行，不占用 Tokio worker。CLI 以 core、media、render 为真实 composition input；`clap` 只负责参数解析，`sha2` 只负责流式 SHA-256，`tempfile` 只负责私有生命周期，`serde_json` 只解码 Rust-owned manifest，Tokio 只负责有界 process/render async work。这些依赖都不能进入纯 core。

`evals/` 是 checked-in 的语言产品证据，不是 runtime package，也不是 CI 中调用在线模型的服务。它拥有冻结的题目、prompt、grader 规则、原始输出、模型参数和对照 baseline。只有真实实验材料可用时才加入这些资产；仓库不创建空框架，也不凭记忆伪造历史 baseline。

`onmark-media` 必须独立而不能藏在 render feature 中，因为“无 Chromium 的素材探测服务”是明确消费者，同时满足依赖预算和独立消费两条判据。Feature 只表达同一包内正交能力，不能用来遮住真实存在的架构边界。

Render Graph 和 planner 在第二关先作为 `onmark-core` 模块加入。只有出现独立消费者、编译成本或清晰发布边界后才考虑拆 crate。worker 状态机先属于 `onmark-render`；coordinator 是第三关的部署系统，不提前进入核心 workspace。

### Core 内部依赖也必须执法

合并成一个 crate 不等于允许模块互相穿墙。`onmark-core` 的内部 DAG 为：

```text
compiler ──→ syntax ──────→ model
    ├────→ diagnostics ───→ model
    ├────→ timeline ───────→ model
    └────→ model

render_graph ─→ timeline / model

protocol ─→ diagnostics / timeline / model
```

箭头表示“左侧可以依赖右侧”；精确允许边如下：

```text
model       → (none)
syntax      → model
diagnostics → model
timeline    → model
render_graph → timeline + model
compiler    → syntax + diagnostics + timeline + model
protocol    → diagnostics + timeline + model
```

`syntax` 不得依赖 compiler，`timeline` 不得依赖 syntax，领域模块不得反向依赖 protocol。CI 使用 `syn` 对显式 Rust path 做语法感知检查。这是一条协作式护栏，覆盖普通路径、import、alias 和 re-export，但不覆盖宏内部生成的路径，也不等价于 rustc 的完整名字解析；这些边仍由评审负责。任何新增内部边必须先更新本文。

`onmark-core` 只允许 `syntax` 使用 `xmlparser` 做纯计算、保留 span 的 XML-compatible fragment tokenization。树构建、嵌套检查、重复属性检查、引用解码和全部创作语义由 Onmark 自己拥有；parser error 在 syntax 边界翻译，该依赖不执行 IO。测试 target 可以使用 `proptest` 验证时间代数，并使用 `syn` 执行协作式模块依赖律检查；二者都不会链接进库消费者或运行时产物。

`protocol` 模块使用 `serde` 定义稳定的 browser 与 bundle-manifest JSON 边界。其可选的 `schema` feature 只为仓库生成工作暴露 `schemars`，产品 binary 不启用它。所有仓库专用自动化统一放在 `scripts/`；它既不是产品 package，也不是杂项应用层。其中的 Cargo manifest 只用于给 Rust schema generator 一份固定的 build-only 依赖预算和稳定的 `cargo xtask` 入口。这个 binary 只由开发者与 CI 消费，只允许依赖启用 `schema` feature 的 core、`schemars` 与 `serde_json`；任何产品 crate/package 都不得反向依赖它。相邻的 Node generator 可使用固定版本的 schema-to-TypeScript 与验证工具链。`cargo xtask schema` 先写 versioned schema，再调用该 generator；`json-schema-to-typescript` 把 browser 类型生成进 runtime、把 manifest 类型生成进 bundler，Ajv 在构建期生成 standalone browser validator，TypeScript 会检查两个生成消费者。Oxlint、窄范围 repository-shape check 与 Prettier 只作为仓库开发门禁，绝不进入 browser artifact。浏览器 runtime 不在运行期动态编译 schema。精确工具版本由 lockfile 与 `mise.toml` 固定，CI 会拒绝过期生成物。

`onmark-media` 只依赖 core，以及用于私有 ffprobe response 边界的 `serde`/`serde_json`。它使用参数数组直接启动配置的 ffprobe executable，绝不经过 shell；退出后仍让派生进程持有输出 pipe 的 wrapper 不属于该 executable contract。在这条 direct-child 契约下，进程寿命和保留的 stdout/stderr 字节数都有显式上限，两条 pipe 并发排空；显式 shutdown 会报告 process-control failure，`Drop` 只作 best-effort termination fallback。私有 ffprobe response type 只在此边界翻译一次并产出 core-owned `AssetMetadata`；JSON value 与第三方 error type 不定义稳定 API，但底层 error 会通过标准 source chain 保留，供调试使用。Gate 一对每条 stream 请求有界的 stream-level facts：`codec_type` 记录音轨存在性并选择第一条视觉流，`nb_frames` 识别 still。仅当 ffprobe 中可解析的 `avg_frame_rate` 和 `r_frame_rate` 约分为同一个精确有理帧率时，才把视觉流归为 constant；二者不一致或不可用时保守归为 variable。因此一 MiB stdout ceiling 与媒体时长无关。

`onmark-render` 拥有 Chromium 与 `FFmpeg` 的重型依赖预算。它只把
`chromiumoxide` 用作 CDP transport 与进程启动器，把 `png` 只用于把 capture
screenshot decode 成 renderer-owned canonical RGBA fingerprint；`tokio` 和
`futures` 也只存在于这条异步执行边界。`tempfile` 为每个 browser session 提供隔离
profile、创建同文件系统的私有输出暂存目录，并保有一个 RAII 私有 unit
root。

unit-root materialization 只用 `serde_json` 编码 Rust-owned manifest、用
`sha2` 流式复核 identity、用 `url` 构造 browser entry URL。file bound 会在
identity 工作前拒绝，canonical hash 与 manifest size 都通过固定内存 writer
流式计算，pretty manifest 直接写入私有 root。它拒绝 symlink 与非普通文件，
复制已验证字节而不链接可变 source path，同时限制保留文件数与总字节。固定
safety envelope 是十万个文件与一 TiB，每个调用方仍须提供更小的显式
policy。因此并行 session 既不共享 Chrome lock，也不共享已接纳的输入路径；
只有 Chromium 与 `FFmpeg` 都干净结束后，才用 no-clobber hard link 发布完整
MP4。

crate 显式提供 executable path、viewport、browser process/request deadline、
encoder inactivity timeout、frame/input/capture byte ceiling、有界 stderr
保留与 shutdown，并把 Chromium、CDP、subprocess 类型翻译成 render 自己拥有
的稳定错误。有限 frame/byte budget 与每次 write、finalization 的 timeout
共同约束 encoder 生命周期；等待 Chromium 的时间不消耗 encoder inactivity
budget。浏览器导航会分别等待 document load 与 runtime host；不能把
transport 的 navigation 返回误当成完整生命周期屏障。

Gate 一每次只拥有一张 PNG，捕获后直接写入 `FFmpeg image2pipe`，不存在 frame
queue 或整段视频 buffer；固定的 H.264 `yuv420p` profile 会在进程启动前拒绝
奇数 viewport 尺寸。conformance 会启动固定版本的 Chrome for Testing 与
`FFmpeg`，加载 production presentation adapter，走过类型化
`Load`/`Prepare`/`Seek` 握手，probe 最终 H.264 MP4 并验证 decoded motion。
checked-in bundle fixture 携带真实 payload bytes，由 bundler test 逐字节重建，
并通过 native materialization 穿过生成的 Node/native manifest contract。最外层
CLI conformance 会启动两次独立的 whole-film session，比较完整的 decoded
raw-frame hash 序列，再验证 no-clobber 发布。CI 显式拥有这些测试使用的 browser
与 media-tool 版本；本机运行仍保持 opt-in，因为它需要这些外部进程。

GitHub-hosted Ubuntu 无法向安装的 Chrome for Testing binary 提供可用的 Chromium
sandbox。因此 real-process job 会提供一个 runner-local launcher 来追加
`--no-sandbox`；这个例外只属于一次性的 CI worker。产品与本地 browser launch
默认仍然启用 Chromium sandbox。

Gate 一的 native browser operation 与 decoded-video wait 最多接受一天 deadline，使所有平台 timer 都处于显式支持的时间范围内。

校验失败原因保留为局部领域值。syntax 提供源码上下文后，由 `compiler` 模块唯一负责把 `InvalidNodeId` 等原因翻译成带源码位置的 `Diagnostic`，包括各阶段特有的 message 和 help；`diagnostics` 只拥有通用诊断表示与稳定 code。`model` 和 `syntax` 都不依赖 diagnostics，调用方也不得重复实现这层翻译。

### TypeScript package 方向

```text
@onmark/runtime  ←  @onmark/authoring
       ↑                  ↑
       └── @onmark/bundler ┘
```

`runtime` 是浏览器底座和长期稳定扩展点，拥有当前帧 hook、FrameReady 协议和 adapter
contract。`stateless/warmup/sequential` 目前只是架构分类，不是公开 capability
declaration；该 API 成为现实后也只能由 runtime 拥有。`authoring` 只通过 runtime 的
types-only entrypoint 使用公开类型，创建语义化 video/overlay DOM，并把 CSS 与 layout 留给
presentation entry。`bundler` 注入固定 authoring/runtime artifact 并生成 manifest；runtime
永不依赖 authoring 或 bundler。Gate 一的 `RuntimeSession` 拥有 protocol 顺序、interval
关系检查、精确帧投影与 terminal disposal；并发 command 直接拒绝，不暗中增长队列，adapter
只会收到递归冻结的 plan snapshot。浏览器具体工作只通过一个窄 adapter 进入，其等待必须
有界，预期失败必须类型化。production presentation adapter 接收 presentation-owned
element、source 与 visibility effect；它负责有界媒体加载、精确 source-frame selection、
decoded-frame readiness、已求解 overlay visibility 与 terminal cleanup，但不创建 layout 或
canvas state。adapter 与 bundler 使用的 materialized asset directory 同样由 Rust bundle
schema 生成。

`@onmark/bundler` 是 Node-only 的产品构建边界，不是仓库自动化。它只允许依赖 Node built-in、`@onmark/authoring`/`@onmark/runtime` 的公开入口和固定版本的生产依赖 `esbuild`；浏览器 package 不得反向依赖它。Gate 一只编译单个 ESM presentation、替换为固定 authoring/runtime 入口、生成固定 document shell，并以稳定 SHA-256 manifest 记录每个 presentation payload 文件。package 通过窄 `onmark-bundle` executable 暴露同一个操作，native CLI 因而不 import Node 或 esbuild type。child process 只接收显式 entry、output 和 retained-byte-limit 参数，成功时不向 stdout 写 payload，失败时向 stderr 写稳定类别；native caller 自己施加 process deadline，持续排空诊断但只保留有界 tail，并把产出的 manifest 重新交给 Rust-owned wire type 解析。manifest shape 与 layout constants 都来自 Rust protocol contract 的生成结果，不在 TypeScript 手写第二份。构建显式限制最终保留字节数，经输出目录同级的私有 staging directory 写入，并拒绝构建前或发布前已存在的输出路径。最后一次 directory rename 能防止读者看到正常完成过程中的半成品，但 Node 的可移植文件系统 API 无法把此前的 absent check 变成跨进程 no-clobber transaction。当前边界刻意不提供 watch、plugin API、cache、development server 或 asset materialization policy。Esbuild 内部工作内存仍由固定的第三方实现管理，不受 retained-output ceiling 约束。

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

Rust wire types 是 source of truth。`cargo xtask schema` 从它们生成 versioned JSON Schema 和 TypeScript types/codecs，CI 重新生成并要求工作树零 diff。生成结果提交进仓库，供 npm package、diff review 和非 Rust 消费者直接使用；禁止手工修改。Gate 一首次对外发布之前，v1 可以原地收口，避免初始公开契约背负实验字段；一旦发布，任何不兼容 wire 变化都必须使用新 protocol version 并带 migration/conformance fixture。Rust 本身直接使用原始领域/wire types，不再从 schema 反向生成第二套 Rust 类型。

`BrowserPlan` 现在携带 production presentation adapter 已真实消费的 output frame rate、evaluation/output interval、primary-video placement，以及 title/call-to-action overlay。video placement 记录 immutable asset identity、绝对可见区间和验证 decoded-frame selection 所需的 admitted CFR source rate；overlay placement 只记录语义角色、decoded text 与 compiler 已求解的绝对区间。materialized URL 仍是 render-owned fact，DOM 结构与 CSS 则始终是 presentation-owned effect。这是一条 Render Unit 的 browser-facing projection，不是 Render Graph 或 partition plan 本身。它只能包含浏览器真实消费的事实；output path、cache key、FFmpeg 参数、source span 和 materialization policy 都不得进入。VFR timestamp map 与更多 component 事实等 production adapter 真正消费时再加入，不提前把后续 gate 塞进协议。

Protocol V1 最多携带 10,000 个 video placement 与 10,000 个 overlay placement；每条 overlay inscription 最多包含 65,536 个 Unicode 字符。一条 failure 最多包含 4,096 个 message 字符与 256 条 pending-resource description，每条 description 最多 1,024 个字符；它们的确定性顺序由 producer 拥有。runtime-host property name 与这些 resource limit 都从 Rust-owned schema metadata 生成，native executor、browser runtime 与 validator 不得各自保存手写副本。

authoring API 可以追求浏览器端人体工程学，但不能复制求时语义。

```text
Rust wire types → checked-in versioned schema → generated TypeScript codecs

handwritten TypeScript authoring API → screenplay source → Rust compiler
```

## 10. 产品表面与可观测性

Gate 一唯一承诺的命令是：

```text
onmark render film.onmark -o film.mp4
```

`check`、`compile`、`inspect` 属于后续可能从真实使用中长出的产品表面；Gate 三当前唯一的 `worker` 表面是同一执行器的窄部署适配器 `worker capture`，不是 coordinator。其余命令都不是当前 CLI 合约，也不能提前生成空命令或 coordinator 脚手架。

Rust API 用于嵌入服务端；TS API 用于 authoring；跨进程使用 versioned schema，不直接暴露内部领域对象。CLI 输出、诊断码和 Execution Plan 都是稳定产品协议。

每次 render 有 render ID，每个 unit 有 attempt ID。Trace 贯穿 compile、bundle、schedule、prepare、capture、encode、upload 和 assemble。核心指标包括单帧 capture/encode 时间、CPU/RSS、channel 深度、缓存命中、重试阶段、网络字节、临时盘峰值和 planner 估算误差。

## 11. 安全边界

用户 HTML/JS 是不可信代码。生产 worker 运行在隔离容器或 microVM：无宿主凭据、默认断网、只读 bundle、限定素材目录，并限制 CPU、内存、PID、磁盘和时间。

不能因为容器启动困难就关闭 Chromium sandbox。FFmpeg 参数使用数组而非 shell。远程素材下载处于独立 fetch 边界，限制 URL、重定向、大小和类型。

## 12. 三个交付关卡

### 第一关（已完成）：稳定渲出一条真视频

唯一目标是证明核心闭环：

```text
Screenplay → Timeline IR → Browser Runtime → Chromium → FFmpeg → MP4
```

范围只有：最小剧本语言、冻结素材 catalog、素材探测、Rust 时间求解、versioned Timeline IR、不可变 presentation bundle、TS 确定性时钟、FrameReady handshake，以及单进程 whole-film Render Unit 的真实视频。Gate 一验证了视频 seek、异步稳定、捕获方式和音频 mux；字体与图片的更多组合仍是 presentation 能力实验，不构成已冻结的 Gate 一语言表面。Gate 一执行并 mux 作者写下的 voice-over，不能静默丢弃音频。

退出一致性测试会构建 release CLI，让同一份剧本经过两组独立 Chromium/FFmpeg session 渲染，比较解码后的音视频逐帧 hash，验证画面运动与 stream facts，并证明最终发布不会覆盖已有输出。

这一关不建设 coordinator、lease、远程 worker、能力调度和分层缓存。

### 第二关（已完成）：正确地切开并总装

已完成的切片把同一影片编译成两个独立的本地 Render Unit，经既有 executor 捕获并总装。退出一致性测试会让同一条含媒体的两镜头影片分别作为 whole-film unit 与两个独立 materialize 的 unit 渲染，再比较解码后的视频与音频 hash，以及首个音频 packet 的落点。它实现 Render Graph 与 `evaluation/output` 区间。转场预卷、持久 unit cache 和依赖闭包失效要等真实依赖或缓存消费者出现后再实现；不提前搭它们的空架子。

### 第三关：离开本机仍然成立

先证明独立 worker process 会发布并 assemble 有界、可校验的 frame artifact，仍复用既有 encoder 与 audio path；再在这个已证明的 interchange 外加入 object storage、lease、retry、幂等发布与 capability scheduling。验收 IR/plan byte-identical、锁定环境下 raw frame hash 一致，以及解码后音视频等价；不预设 MP4 容器字节必然一致。

每一关都使用最终方向的 IR 和协议，但只实现本关真实消费的部分。上一关没有稳定通过，不创建下一关的空架子。

## 13. 待实验决策

Gate 一首轮 capture spike 已得到正向但刻意收窄的证据：页面自行控制 `FrameReady`，随后调用 CDP `Page.captureScreenshot`，DOM/CSS/Canvas 帧在同一锁定机器的独立 Chrome 进程间得到一致的 raw RGBA hash。这只决定下一轮实验路线，不等于最终 transport contract；decoded media、WebGL、异步组件、跨环境一致性与生产级 lifecycle 仍未证明。

decoded-media 实验现已覆盖 30 fps CFR、`30000/1001` CFR 与交替帧间隔 VFR H.264；三者都使用 30 帧 GOP、3 个 B-frame，并按 `17 → 3 → 29 → 17` 乱序 seek。只有在 `requestVideoFrameCallback.mediaTime` 确认 output-frame 中点实际选中的 source frame 后才返回 FrameReady；VFR 期望来自 ffprobe 的真实 source-frame timestamp，不假设 source/output frame 对齐。两个独立 Chromium session 的 PNG capture byte-identical，同一 source-frame timestamp 的独立 FFmpeg extraction 也在重复执行间 byte-stable。实验同时发现：把精确 CFR 帧边界秒数直接写入 `video.currentTime` 会选中前一帧，必须采样 Rust 已选帧内部。

两条 decode path 并非 pixel-interchangeable。四张 320×180 RGBA 帧共 921,600 个 channel，Chromium canvas 与 FFmpeg raw extraction 约有 229k–232k 个 channel 不同，mean absolute delta 为 2.13–2.18，孤立最大值为 173–178。当前机器上 browser seek/readiness/screenshot 平均 51–81ms/帧；每帧单独启动 FFmpeg 的 native extraction 为 18–19ms，但后者尚未包含 browser injection、composition 与最终 capture，因此不能当成端到端速度胜负。Gate 一的一次 render 必须只认一条 decode/color path，并把它纳入锁定环境；多 codec/色彩、更长随机序列、persistent native decoder 成本与 injection overhead 仍需测量。

因此 Gate 一只接纳 CFR H.264 视觉素材，并把锁定 Chromium decoder 作为唯一权威 decode/color path。adapter 在 Rust 已选帧内部采样，且只有 `requestVideoFrameCallback.mediaTime` 指向期望 source frame 时才返回 ready。不支持的 codec 或 VFR 必须在 render 前显式拒绝，不能近似执行。只有 frozen metadata 与 Browser Plan 将来携带完整 timestamp map、而非单一 CFR rate 后，VFR 才能转为正式能力。FFmpeg exact-frame extraction 保持备选实验，不作为会在同一次 render 中改变像素的隐藏 fallback。

这条策略由 render-owned `AdmittedVideo` proof 对 core-owned metadata 执行 admission 来表达。它借用规范化事实，不再复制一套 render 媒体模型，并证明 H.264 codec 与唯一精确 source frame rate。whole-film Render Unit 保留该 rate，并只向 browser placement lower 一次。decoded-media conformance 通过生产用的有界 ffprobe 边界，为两个被接纳的 CFR fixture 和一个被拒绝的 VFR fixture 生成 proof。whole-film executor 通过 production adapter 消费被接纳的视频，并验证最终动态画面产物。

- Chromium 控制选择 CDP、WebDriver BiDi 还是极薄现有库；
- 捕获选择 BeginFrame、screenshot、surface copy 还是编码流；
- 分层 alpha 缓存何时值得额外成本；
- Execution Plan 公开编码使用 JSON、Protobuf 或分层组合；
- coordinator 首版存储使用 SQLite/Postgres 还是可替换 trait；
- 哪些动画适配器可随机 seek，哪些必须 warm-up/sequential；
- 浏览器、字体与 FFmpeg 环境锁定到什么粒度。

实验优先级依次为：捕获方式与 FrameReady 正确性、未知组件的保守执行成本、分片与预卷、跨 worker 一致性、分层缓存收益。纯编译内核、确定性协议、依赖驱动分片和本地/分布式同构是基础骨架，不应反复摇摆。
