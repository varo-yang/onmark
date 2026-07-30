# Onmark 架构设计

> 状态：Gate 八已在 Gate 一至 Gate 七及分布式增量渲染完成后启动。已完成关卡保留为历史验收证据；尚未实现的能力会明确标为延期。

本文与《Onmark 语言规格书》平级。语言规格负责“创作者如何表达影片”，本文负责“已编译的影片如何成为成片”，两者只通过 versioned
Timeline IR 接合。

```text
Language Specification                     Render Architecture
screenplay → semantics → diagnostics       render graph → execution → artifacts
                    └──── Timeline IR ────┘
```

文中内容分为三个成熟度：

- **基础原则**：现在写进代码，并保持稳定；
- **已完成关卡**：保留当时的验收范围与证据，不冒充当前限制；
- **延期能力**：尚未实现，只有在指标和真实负载出现后施工。

## 1. 系统定义

Onmark 是一个以剧本为源语言、以浏览器为画布、以确定性 Render
IR 为执行合约的视频编译与渲染引擎。

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

`<om-scene>`、`<om-shot>`、`<om-vo>`
和 cue 是创作意图；绝对帧区间、依赖边、缓存键和渲染分片是编译事实。两者不能混成一个万能
`Document`。

### 编译器是纯函数

相同文档、素材元数据、编译选项和版本必须产生 byte-identical
IR。编译器不访问网络、不生成素材、不读墙钟，也不启动浏览器。媒体探测属于编译前 IO，其结果作为显式输入。

### 远程执行不是另一套渲染器

CLI 与远程 worker 执行相同的 Render Unit 合约
和 worker 状态机。本地父进程与短生命周期远程 invocation 只是用不同方式拥有同一份有限 DAG，不能有日后删除的“简化渲染路径”。

### 分片由像素依赖决定

`shot`
是优秀的创作和缓存候选边界，却不是无条件执行边界。只有 presentation 显式声明并证明
random access，graph 才把各 shot 记录为独立 region；未知 presentation code 仍形成一个
sequential region。显式 transition 会增加同时依赖两个相邻 shot 的 overlap region。
graph 为每个 region 记录精确 shot identity 与直接冻结媒体依赖。这不是“shot 天然可切”的
通用规则。贯穿元素、全局层、shader history 或相邻采样依赖仍未实现；这些能力必须先扩展
依赖表示、扩大或合并 region，才能进入分片。

### 浏览器只负责画，不负责决定

Chromium 不求时间、不发现素材、不选择分片。它只接收已求解的帧号、场景状态和资源清单，在唯一主时钟下画出一帧。

### 每个昂贵结果都有明确身份

冻结素材和 bundle payload 用内容哈希；渲染单元与任务用 canonical input
identity；worker frame artifact 用 capture-contract identity，并保留内部 payload
checksum 和 raw-pixel evidence。capture-contract identity 同时提交 browser plan、
bundle、render profile、实际选择的 visual-execution path 与 locked capture environment；
缓存正确性来自这些可验证身份，不来自文件名约定。

## 3. TS 与 Rust 怎么分

边界沿“浏览器世界”和“确定性系统世界”切开：

| 领域                            | TypeScript        | Rust                    |
| ------------------------------- | ----------------- | ----------------------- |
| 剧本 authoring 类型与组件 API   | 主责              | 验证最终文档            |
| DOM/CSS/Canvas/WebGL/Three.js   | 主责              | 不重写浏览器绘制        |
| 浏览器主时钟、seek、frame-ready | 主责              | 发命令并校验协议        |
| WAAPI/GSAP 等动画适配器         | 主责              | 消费能力声明            |
| TS/JS bundling                  | Node/esbuild 适配 | 生成 manifest、启动工具 |
| Parse、名称绑定、时间求解       | 不重复实现        | 主责                    |
| Typed IR、Render Graph、分片    | 只消费协议        | 主责                    |
| 缓存键、调度、幂等、重试        | 不负责            | 主责                    |
| Chromium/FFmpeg 生命周期        | 浏览器内应答      | 主责                    |
| CLI、worker、短生命周期编排     | 可提供 JS wrapper | 主责                    |

Rust 编译器不是为了“HTML 解析更快”，而是因为它是系统信任根：phase
type 固化 Parse → Structural Bind → Resolve →
Solve，newtype 区分帧号、帧数和时间基，enum 穷尽时间规则与诊断，同一内核可直接嵌入 CLI、worker 和短生命周期编排入口。验证发生在第一次拥有足够信息的相位；Solve 直接构造 Timeline
IR。没有新表示去证明新不变量时，不添加仪式性的 Validate/Lower 相位。未来需要浏览器调用时，可以从同一内核构建 WASM/N-API
binding，不能维护第二份求时逻辑。

## 4. 六种核心表示

```text
Source AST
  → Structurally Linked Film
    → Resolved Film
      → Timeline IR
        → Render Graph
          → Partition Plan
```

### Source AST

保留源码结构、属性原文和 span，用于精确诊断。允许未知标签、错误引用和未解析时间。

### Structurally Linked Film

元素词汇、合法包含关系与 film 全局 ID 已完成绑定；尚未解析的 authored
attributes 仅作为 compiler 私有相位输入保留。

### Resolved Film

duration、cue、素材引用与内容起点已变成带 source
span 的类型值，不再向公有 API 暴露 syntax-layer attributes。

### Timeline IR

所有时间规则已经求解成准确区间：

```rust
pub struct TimelineTiming {
    interval: FrameInterval,
    start_reason: TimingReason,
    end_reason: TimingReason,
}
```

每个 Timeline 元素保留这份 timing fact。使用整数帧或有理时间基，禁止裸
`f64`。区间的两个端点各自保留“为什么在这里”的原因，服务诊断、调试和增量失效；这些是compiler
fact，不会把 `start`、`end` 或 `begin` 属性带回 screenplay 语言。

### Render Graph

Timeline IR 回答“何时存在”；Render Graph 从已求解事实和已接纳的时间能力中推导可独立
求值的 region，并记录其精确 shot identity 与直接冻结媒体依赖。普通独立 shot 的
evaluation/output 相同；显式 transition 会把边界拆成三个互不重叠的 output region，并让
中间 overlap region 同时 evaluation 两个相邻 shot 的完整区间。persist、全局效果与历史
采样边仍未实现；未来能力若产生这些依赖，必须先扩展图并扩大或合并 region，才能进入分片。

### Partition Plan

这是 core 内的纯分片事实：每个 `RenderPartition` 记录 `output`、`evaluation`、精确 selected
shot identity，以及分配给该候选 unit 的冻结素材。它不拥有路径、browser URL、bundle、
进程配置或云厂商类型。`output` 是最终提交的帧；`evaluation` 可以因已证明的依赖扩大，但
worker 仍只发布 `output`。

编译管线在 Timeline IR 结束，执行管线从一条独立的组合边界开始：

```text
Partition + Timeline IR + Frozen Asset Catalog + Bundle Manifest + Render Profile
  → Render Unit
    → Browser Plan + Visual Execution Plan + Audio Plan
      → materialize → Executable Unit + verified private root
```

这条接缝不是另一个编译相位。Timeline
IR 只回答影片中什么事实在何时成立；presentation
bundle 负责把这些事实画成 DOM、CSS、Canvas 或 WebGL；Render
Unit 则定义一次 executor 调用消费哪些不可变输入。whole-film render 会直接组合一个 unit；partitioned render 从每个 `RenderPartition` 组合相同类型的 unit，不更换执行器合约。

Gate 一最初的 `AudioPlan`
用已求解的旁白 placement 建立了原生混音边界。materialization 会把冻结音频字节与浏览器素材一起复制，却不把它们变成浏览器输入。Chromium 编出视觉流后，executor 混合轨道并将 AAC
mux 进最终 MP4。每条 unit 和完整装配序列最多保留 32 条音轨，使 `FFmpeg` 进程、输入描述符
与 filter graph 边界始终有界。Gate 四只扩展同一边界上的 fact 与 sample policy，不另造第二套
audio engine。

Gate 二的首条本地组装路径会在各自独立 materialize 的 unit 依次把连续 output 帧送入同一个视觉 encoder 期间保留这些 unit
root。音频 placement 先保留绝对 Timeline 起点，最终总装时只按成片 output 原点重基一次，并在所有 unit 的画面都捕获完后混合。这样既不假定已 mux
AAC 的分段可以安全拼接，也不再做一次有损的视频重编码。这是一条正确性优先的路径，不是持久分段缓存格式；缓存编码分段必须先有独立的等价性证明，才能成为执行产物。

Gate 四保留 voice-over 的 narrative Timeline 节点，同时把其可执行的 asset、interval、gain 与 role
收敛到 music 和 sound effect 共用的 `TimelineAudio` fact。通用音频是 film-level collection，因为一条 music
bed 可以跨越 shot 与 partition。Render Graph 把每条 placement 交给其起点所在的唯一 region；该 owner
只 materialize 一次冻结字节，但 placement 可以越过 owner 的 visual output，并且只在最终总装时混合。

音频探测现在保留 selected stream 的正整数 sample rate 与归一化的 mono/stereo channel layout；其他声道数会在 FFmpeg 隐式 downmix 之前被拒绝。mono 会被显式复制到双声道，stereo 保留左右声道身份，AAC 编码前的固定 mix profile 是 48 kHz stereo floating-point audio。unit composition 将精确帧长度用具名的向上取整策略只投影一次到该 sample
grid：时间戳仍早于 Timeline exclusive end 的 sample 会被保留。每条输入先在 source grid 上 trim，再重采样到固定 48
kHz mix grid。Rust 用向上取整把 frame start 投影到该 grid，因此 `FFmpeg` 收到整数 `adelay` sample count，而不是自行计算 decimal 或 floating timing expression。canonical rational linear
gain 通过 `volume` 应用；`amix` normalization 被明确关闭，避免多轨重叠时静默改写 authored gain。最终 AAC path 按 visual frame count 在同一 output sample grid 上的投影 trim 或 pad 混音，再由 visual stream 通过
`-shortest` 封口容器。因此跨 partition 的 owner track 不能让单独渲染的 unit 长于其 visual output。screenplay
Timeline IR 还把每条已接纳 fade 保留为精确 frame count。encoder 把 fade-in end 与
fade-out start 投影到同一个 48 kHz grid，并生成显式指定 linear curve、silence 与 unity
level 的 sample-indexed `afade` filter。fade-out rounding 由 placement end 独占，因此在
frame grid 上刚好相接的 ramp 不会因为两次独立 sample rounding 而重叠。
已入库的 audio-syntax eval 用四十条真实模型输出比较了语义元素 `<om-music>`/`<om-sfx>` 与泛化的
`<audio kind="...">`。两臂都保持 20/20 generation reliability，因此 Gate 四接纳语义元素：元素类型直接编码 role 与合法 containment，不再引入第二套 kind/parent
有效性矩阵。authored gain 是 `0%` 到 `100%`（含端点）的精确闭区间。

## 5. 从源码到 MP4

### A. 装载并冻结输入

Loader 接收项目根、入口和渲染参数，解析本地引用并生成不可变输入清单。远程 URL 必须先下载进内容寻址素材库；编译和渲染不直接依赖会变化的 URL。

素材在三层身份之间显式转换，不能混用：

- `AssetRef` 是剧本中作者写下的逻辑引用；
- `FrozenAssetId` 标识实际被探测、被编译的不可变字节；
- materialized asset 是 worker 为同一份字节准备的本地路径或 browser URL。

Loader 或 composition root 先计算并验证
`FrozenAssetId`，probe 读取同一份已冻结字节并产生 `AssetMetadata`。Compiler 接收
`AssetRef → (FrozenAssetId, AssetMetadata)` catalog，Timeline IR 只保存
`FrozenAssetId`，绝不保存可变路径，也不把作者拼写误称为冻结身份。执行前的 materialize 再把冻结身份解析成 worker-local
location，并复核 digest。

第一关的 `FrozenAssetId` 固定使用 SHA-256，canonical spelling 为
`sha256:<lowercase-hex>`。hash 计算属于 IO freezing
boundary；core 只拥有已计算的身份与确定性映射，不读取文件。

### B. 探测素材

Probe 使用 ffprobe 或原生解析器提取 duration、codec、尺寸、帧率、色彩信息和音轨布局，输出规范化
`AssetMetadata`，并按素材 hash 缓存。

### C. 编译

```text
parse → bind structure → resolve attributes/references → solve Timeline IR
```

创作错误产生可聚合 diagnostics；机器故障返回 typed
error。编译成功保证时间线唯一、自洽，但不意味着浏览器已经可执行。

结构 bind 与属性/引用 resolve 都会在构建候选产物的同时聚合创作诊断。只要存在 error，相位报告就不公开对应阶段值，避免被拒结构或恢复默认值被下一阶段误当成编译事实；warning 不阻塞产物。

Timeline solve 消费由 `onmark-core` 拥有的 `AssetRef → FrozenAsset`
catalog；其中 `FrozenAsset` 绑定不可变身份与同一字节产生的规范化
`AssetMetadata`。`AssetRef` 是 screenplay-relative portable path，只允许 `/`
分隔，不能是绝对路径，不能含
`..`、空组件、`.`、反斜杠或平台前缀。metadata 记录精确素材时长，以及选中的音频和视觉流各自的精确 stream
duration；视觉流还会记录 codec、pixel format、正的 source-pixel dimensions、完整且已识别的
color tuple，以及一个精确有理帧率或 variable timing。单帧流会单独建模，因为确切的单帧计数无法证明 source rate。`onmark-media`
通过探测生产 metadata，loader 或 composition
root 负责冻结同一份字节；ffprobe 专属结构、路径与失败不得进入 core。引用素材若不在 catalog 中，属于 typed
integration failure，而不是 authored diagnostic。媒体元素缺少 authored
source 时仍可通过静态 resolve，但无法产出可渲染 Timeline
IR，并在 solve 阶段收到 authored asset diagnostic。

诊断是语言产品的一部分，不是日志。每条创作诊断必须包含稳定 code、源码 span、直接原因、相关节点，并在存在确定修法时给出可执行建议。建议面向人和 LLM 使用源码词汇，例如“定义
`cue:offer`，或将该标题改为相对当前 shot 的 `delay`”，不能只暴露求解器术语。

### D. 构建 browser bundle

Bundler 把用户组件、Onmark
runtime、CSS 和静态依赖打成不可变 bundle。bundle 只包含绘制能力，不包含时间求解逻辑。manifest 记录固定 entry point、document scope、temporal/visual capability、frame behavior 和每个 payload file；runtime、字体与静态依赖通过这些已哈希文件进入 identity，不另建可变 metadata。这四项声明都进入 `bundleId`。紧凑 UTF-8 JSON identity 是
`{version,entryPoint,documentScope,temporalCapability,visualCapability,frameBehavior,files}`；file 按 portable path 排序，每个 identity entry 的字段顺序固定为
`{bytes,path,sha256}`。这是 versioned contract，不是 pretty-printed
manifest 的偶然表现。bundle 是可重建的临时产物，reader 只接受当前版本，不保留旧版兼容分支。manifest 包含一到 99,999 个 payload
file；path 只能使用小写 portable ASCII，最长 1,024 bytes，不能进入 unit-owned
namespace，也不能让一个 file 成为另一个 file 的目录祖先。其余字段只在 authoring 或 execution 真正消费时加入。

bundle 保留 presentation-owned HTML，并安装把已求解 fact 绑定到 semantic custom
element 的 infrastructure。在计算 bundle identity 前，Node boundary 会移除已由
Timeline IR 或 Browser Plan 表示的 cue/audio element，以及 `src`、`duration`、`delay`
和 `cue` attribute；ID、class、普通 attribute、嵌套标记、inline style 与 overlay 文本
仍是精确 browser 输入。这样，只修改既有 audio 或 compiler timing fact 的值或内容不会
改变视觉 presentation bytes；这并不声称任意 source restructuring 或 presentation edit
都能局部失效。inline CSS 与可选 motion module 拥有 presentation；infrastructure 不提供
template、layout、style、animation 或 full-screen assumption。所有文档共用同一套
确定性时钟、readiness 和媒体原语。作者侧浏览器代码的公开规则写在
[presentation contract](presentation-contract.md)。

materialization 组装一个 content-addressed unit root：所需素材位于 presentation
entry 下的 `assets/sha256/<lowercase digest>`。browser 直接从 `BrowserPlan`
已携带的 frozen
identity 推导这个相对位置，因此不需要第二套 native-path/browser-URL wire
protocol。unit 只在 assembly 前保留 worker-local source
path；materializer 复核精确字节后复制进私有 root，不用 link 把后续 source-path 变化带入执行。`RenderProfile`
拥有 viewport dimension 等会改变 pixel 的事实；process
deadline 与 retained-memory ceiling 仍是 executor limit。materialization 会消费
`RenderUnit` 并产出同时拥有 `BrowserPlan` 与已验证私有 root 的
`ExecutableUnit`，executor 因而不可能把 plan 与无关 URL 或 asset root 拼在一起。

第一关不提前实现 Render Graph。它直接把整部 Timeline
IR、冻结素材 catalog、bundle manifest 与 render profile 组合成一个 whole-film
Render Unit：

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

capture worker 不拥有 visual
encoder；短生命周期 render owner 验证有限数量的 artifact，assembler 拥有唯一一条连续的 visual encode。

### G. 音频和总装

音频不经过浏览器截图。Rust 从 Audio Plan 生成 FFmpeg filter
graph 或 DSP 计划，完成裁剪、delay、fade、gain、重采样和混音。Assembler 验证每份 frame
artifact 的 unit identity 和 capture-environment
identity，再把已验证 PNG 帧流送进一条连续 visual encoder，最后在 assembled
output origin 一次性混音并发布。独立编码的视频段不假定可安全 stream-copy 拼接。

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
FrameStaged(frame_index)
Confirm(frame_index)
FrameReady(frame_index)
Dispose
```

native rendering 只在 Linux `BeginFrame` backend 使用单独发行的
`chrome-headless-shell`，并为每个 render target 启用 CDP BeginFrameControl。
macOS、Windows、普通 Chrome 和 Chromium 使用 portable screenshot path，不请求
BeginFrameControl。
两个 backend 都从同一个 `Load` 开始，为 plan 中的每个 video 与
overlay 创建 binding。inactive
node 保留稳定的 binding identity，但在其 solved
interval 使其可见之前不进入 layout 与 compositor。一个 Render
Unit 未包含的 placement 因而不能扰动它的像素。

`Prepare` 之后，executor 会在固定的 pre-baseline
timestamp 发送并等待一次不带 screenshot 的 visual
`HeadlessExperimental.beginFrame`，用于初始化 page surface。这不是关闭 display
updates 的 warm-up tick：`noDisplayUpdates`
为 false，且命令会在 capture 前完成。真实 capture 从更晚的固定正 compositor
baseline 开始；此后由 session 独占的时钟为每个 capture transaction 前进一个固定正
step。有理帧率仍作为声明的 CDP frame interval，但它与 authored frame 都不决定
transaction identity。authored time 可以后退或重复，Chromium compositor clock
永远不会倒退。`Seek(frame)` 随后应用 browser state、预先注册 decoded-media
observer，并在媒体 seek 完成后返回 `FrameStaged(frame)`，但不等待 compositor
presentation。

在 plan 已知的 video 或 overlay boundary，executor 会先在当前 compositor
transaction capture tick 之前的固定亚毫秒 offset 发送一次无 screenshot visual
BeginFrame，让新可见 layer 获得一次 compositor
turn，同时不推进剧本时间。随后正常的 `HeadlessExperimental.beginFrame`
会在该 transaction tick 提交 frame state 并捕获无损 PNG。

compositor 没有 visual damage 时，headless shell 可能省略
`screenshotData`。native 通常复用上一张有效 PNG，但在 placement
boundary 绝不复用。缺失的 boundary 或首帧 screenshot 会获得一次有界的正亚毫秒 offset 重试；重试仍为空就失败而不循环。

capture 后的 `Confirm(frame)` 会在 native 接受 captured
payload 前等待预先注册的 media observer。在 placement
boundary，observer 可能在 pre-capture commit 上完成。确认后 native 会在该
transaction 的下一个正亚毫秒 tick 执行一次有界的 reconciliation
capture。若没有新的 compositor
damage，就零拷贝复用精确 capture 的 PNG；若出现新 pixels，则用确认后的结果替换。
native 只有完成这一步后才能写入 payload。这样可以关闭 media observer 与精确
screenshot 分别落在同一 compositor turn 两侧的竞态。

portable `Screenshot` backend 不拥有 compositor clock。它在 `Seek(frame)` 完成后激活
target，通过 `Page.captureScreenshot` 读取一次当前 surface，以这次 readback 触发并界定
browser paint；随后仍执行同一个 `Confirm(frame)`。placement boundary 会在确认后再读取
一次，以关闭 decoded-media observer 与首次 surface readback 分居 compositor 两侧的竞态。
普通 frame 不增加第二次 readback。该分支改变的只有 surface commit/readback mechanism，
不会改变 authored frame、runtime state、resource readiness、partition 或 encoding。

该顺序避免三种边界错误：在 BeginFrame-controlled compositor 前等待
`requestVideoFrameCallback` 会死锁；让一个 layer 到 capture
command 才首次进入 compositor 会产生 stale/blank
frame；保留无关的未来 layer 则会使 whole-film 与 partition capture 不同。surface
initialization、placement commit 与 capture baseline
timestamp 都永远不成为调度或协议真值。

portable visual plan 会记录一项经过校验的 browser-capture cadence。`everyFrame`
让每个 output frame 都拥有一次正常 capture command；`placementBounded`
只捕获首个 output frame 与每个已求解 placement boundary，并在 boundary 之间复用
同一份 immutable PNG。renderer 会跳过这些复用帧的 `Seek`、`Confirm` 与 screenshot
工作，但仍把每个 output frame 写入 encoder 或 worker artifact；原生主视频继续通过
现有 compositor 逐帧推进。boundary capture 仍会增加一次 non-capture commit 与一次
post-confirm reconciliation capture；缺失的首帧或精确 boundary screenshot 最多增加一次
有界重试。

cadence 绝不通过 screenshot equality 或 source inspection 推断。bundle 必须声明
`placementBounded`，该声明必须同时具备 `randomAccess`，并且 visual admission
必须证明 Chromium 不拥有 video placement。即使 bundle 授予更强行为，只要
browser-composite unit 含有 video，实际计划仍保持 `everyFrame`。所选 cadence 会与
worker visual plan 一起序列化，并在 materialization 时再次对 bundle 与 browser plan
校验。它与 Chromium 单次 transaction 内的 no-damage response 是两回事：前者是跨帧
计划事实，后者只是对缺失 screenshot payload 的有界处理。

direct rendering 把 PNG 留作 encoder payload；worker
capture 额外把它 decode 成配置 profile 的精确 8-bit RGBA
viewport，并对 canonical pixel bytes 做 hash。worker artifact 会把这个 raw-pixel
hash 和每条有序 PNG record 一起记录，因此可比较独立 capture，而不把 PNG
compression bytes 当作 visual truth。runtime 不发布另一份自行定义的 state
hash。runtime 内部的 `RuntimeFrame`
保留精确整数帧身份，只在调用浏览器 API 时从 Rust 给出的有理帧率推导浮点秒数。超时要指出未稳定资源，不能只报
`page timeout`。

未来的分片可以按组件的时间行为分类：

- `stateless`：任意帧直接 seek；
- `warmup(n)`：输出前需计算 n 帧；
- `sequential`：只能从 checkpoint 顺序推进；
- `global`：影响整个画面；
- `neighbor(radius)`：依赖前后时间窗。

这些是当前的架构分类，还不是公开 capability declaration
API。该 API 成为现实后，Planner 才能根据声明选择分片。**未知组件必须默认
`sequential`，而不是
`stateless`**：可并行性必须被证明，不能被猜测。Onmark 原生动画可天然提供声明；官方 adapter 可为 WAAPI、GSAP 等已验证用法提供声明；用户组件只有经过显式声明和确定性测试才能升级为
`stateless`。自动识别只提供建议，不能静默放宽正确性策略。

重复渲染检测是 conformance
gate，但不能数学上证明任意用户代码无状态，因此不能作为危险默认值的补丁。

### 确定性分层承诺

“确定性”不能笼统等同于“最终 MP4 字节永远相同”：

| 层级                                          | 承诺                                                                                      |
| --------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Timeline IR、Partition Plan                   | 存在 canonical encoding 后，相同输入必须 byte-identical；当前内存表示只承诺结构确定性     |
| 锁定 Chromium、字体、GPU/软件栈后的 raw frame | 目标为 frame hash 完全一致；worker artifact 的逐帧 fingerprint 将其变成可执行的一致性契约 |
| 跨异构机器的浏览器输出                        | 以 conformance 结果定义支持范围，不提前承诺                                               |
| 编码后容器                                    | 校验时间戳、帧数、codec 和解码内容；是否 byte-identical 单独验证                          |

缓存键必须匹配实际承诺的环境边界。不能为了 MP4
metadata 的字节顺序牺牲更重要的画面正确性。

## 7. 远程执行模型

一次远程 render 是由一个短生命周期 invocation 拥有的有限 DAG。父进程或云厂商原生 workflow
可以临时保留进度；worker 直接与对象存储交换 immutable bundle、素材和产物。相同 render
identity 重跑时，会验证并复用已完成的 artifact。因此引擎的正确性不依赖数据库、持久队列、分布式租约或 Redis
锁。未来多租户服务可以在外层增加 admission、计费和账号系统，但它们不是 Onmark engine 的依赖。

分布式增量 capture 有两个有界 admission 点。worker 读取 `request.json` 后，必须在下载
presentation input 或准备 Chromium 之前验证预期 artifact。验证命中时直接返回已有位置，
并明确记录本次跳过 capture；只有 miss 才进入原有的 materialize/capture/publish 路径。
未来若有一个短生命周期调用方已经拥有 deployment 的 artifact namespace，它可以在调用
Lambda 前做同一检查；但 Onmark 不会仅为省掉一次轻量 warm invocation 就引入 caller、
coordinator 或 durable progress state。worker 内检查是 correctness boundary，即使以后
外层裁剪已知 hit，也仍然保留。

Gate 三先采用一个刻意收窄的 interchange：worker 把一个完整计划输出区间捕获为一份有界、带校验和的 frame
artifact。它是单个版本化文件，记录精确 output interval、render
profile、visual-plan 与 locked capture-environment
identity，并携带有序 PNG 流及其 canonical raw-RGBA
fingerprint。worker 在同目录 staging file 中写完后通过 no-clobber
link 发布；重试只能验证或复用同时匹配计划 unit 与 capture
environment 的已有不可变结果，永远不会读到半成品。assembler 会验证每份 artifact 对应其计划 unit 和 capture
environment，再像 Gate 二一样把已验证帧流送进同一个连续 visual
encoder，最后在 assembled output origin 一次性 materialize 并 mix 全部 audio。

artifact checksum 证明 storage integrity；记录下来的 fingerprint 本身不能充当 pixel
evidence。reuse 与 equivalence check 会按顺序解码每个有界 PNG、重新计算 canonical
raw-RGBA fingerprint，并且比较过程中最多只保留一帧 decoded pixels。

这不是 remote-frame queue：一个 worker 独占连续 unit，只有 browser
session 完成后才发布一个 artifact。它也不是 encoded-segment
cache：不能假定独立 AAC-muxed MP4 可以安全 concat；独立 visual
encode 也必须先有单独的等价性证明，才能替换无损 frame
interchange。昂贵且已证明可 random seek 的长场景以后可继续切成连续 frame
range。绝不把单帧做成远程任务。若未来真实负载需要多种 worker，外层编排可按 CPU、内存、GPU、Chromium
slot、encoder slot、codec、磁盘和网络能力选择目标；当前 Gate 三不实现 scheduler。worker 内 browser 数、frame
channel、下载并发和临时盘全部有界。

第一份实现只用 local filesystem 来证明 process 与 artifact
contract。该 conformance 通过后，`deploy/aws-lambda` 成为第一条刻意收窄的 cloud
adapter：它的 Rust binary 只拥有 versioned Lambda invocation/result
JSON、有界 S3 materialization，以及对既有 `onmark-render` frame
artifact 的 conditional publication。它不造 generic object-store
trait，因为真正需要的就是 S3 multipart `If-None-Match: *`
completion、把 precondition reuse 与本次 capture 的 raw-RGBA
sequence 对照验证，以及有界 conflict retry。AWS type 在这个 deployment
package 截止，不得进入 core 或 render。

一次 invocation 用同一个绝对 13 分钟 work
deadline 覆盖 materialization、capture、verification 与 publication。multipart
publication 必须在 upload
owner 内观察该 deadline，因此到期后仍能尝试 abort；若清理也失败，typed
error 同时保留原始失败和 abort 失败，不能让后者覆盖前者。Lambda 平台上限剩余两分钟留给 abort 与 runtime
response delivery。

adapter 会在同一个 Lambda request identity 下，为每个昂贵相位记录一条 structured
completion event，其中包含 elapsed milliseconds 与 success
state。直接同步执行 conformance 时，client 必须关闭自动重试，并把 read
timeout 设得长于 worker deadline；否则 AWS CLI 较短的 transport
timeout 会在第一次 invocation 仍运行时又启动一份昂贵 capture。immutable
publication 仍保持正确，但幂等性不代表可以浪费浏览器工作。

该 adapter 仍没有 coordinator、queue、lease database、全局 retry
owner、capability scheduler、infrastructure definition 或已发布的 Lambda
release。invocation 只能选择 immutable worker-input 的 S3 prefix；output
namespace、browser payload、locked capture-environment
identity 和资源上限都由 deployment artifact 拥有。handler 显式选择
`BrowserLaunchPolicy::isolated_worker()`：process
isolation 归 Lambda 外层边界，renderer 使用实测的 single-process、no-zygote 与 in-process
SwiftShader
topology。它既不是 launch 失败后的自动 fallback，也不能由 invocation 选择。

当前实测首选形态是把 compact browser archive 放进 function
ZIP。handler 在任何 browser IO 之前开始 Runtime API
polling；第一次有界 invocation 校验并展开 payload，后续 warm
invocation 复用同一份私有 installation。一次真实 arm64 Lambda
experiment 使用 92.4 MB function ZIP 与 4,096 MiB 配置，对同一份 title-only
30 帧 320×180 fixture 启动三个独立 cold
environment，端到端分别为 3.005、2.277、3.069 秒，峰值 455–457
MB；紧随其后的 warm reuse 为 1.325 秒。同一 artifact 在 2,048 MiB 的独立 cold
run 为 3.069 秒、峰值 454 MB，在 1,024 MiB 则为 5.080 秒、峰值 451 MB；因此 2
GiB 是这份小 fixture 当前实测的 latency/cost knee，而不是已经冻结的 production
default。对照实验中，相同的 249 MiB expanded browser 位于 fresh container-image
layer 时，`capture_artifact` 需要 30.9 秒；若在 Runtime API
polling 前展开压缩 payload，则仍会耗尽 Lambda 十秒 init
window。这些数据只为该 locked environment 选择 ZIP delivery + invocation-owned
preparation，不能外推到其他 workload。`deploy/aws-lambda` 自己拥有的 reviewed
packager 已取代手工 ZIP 步骤，但 release workflow 与 infrastructure
definition 仍不是 production 承诺。already-expanded
executable 仍是受支持的部署输入。

另一组 decoded-media 实验专门测 steady capture path，而不是 package
delivery。同一份 1,920×1,080 H.264 fixture 以 30 fps 产出 60 帧，当前独立 cold
environment 的 canonical raw-RGBA fingerprint 逐帧一致。单次 warm
capture 在 2、4、8 GiB 下分别为 22.07、13.00、7.91 秒，对应 47.11、58.72、73.46
GB-seconds；峰值实际内存始终为 600–616 MB。因此 memory tier 主要购买 Lambda
CPU：2 GiB 的实测成本最低，8 GiB latency 最低，4 GiB 位于两者之间。8
GiB 下，60 帧合计花费 2.96 秒在 runtime staging 与 media
seek、3.83 秒在 BeginFrame screenshot readback、0.79 秒在 PNG
decode 与 canonical fingerprint；confirmation 与 artifact
write 合计不足 0.2 秒。这些单样本把下一轮性能目标收窄到 seek 与 screenshot
transport，但不冻结 production memory tier。

此前观测到的 66 秒不是 cold-start baseline，而是 correctness failure：旧 frame
handshake 一直等到 deadline，AWS CLI 默认 60 秒 read
timeout 又在首个 invocation 尚未结束时自动重试。直接同步 conformance 必须关闭 client
retry，并让 read timeout 长于 worker deadline。

## 8. 缓存与修改底轨（基础模型，分阶段实现）

```text
Asset metadata
  → Typed IR
    → Browser bundle
      → Render unit artifact
        → Mixed audio
          → Final container
```

Render Unit 缓存键覆盖规范化 plan
fragment、传递依赖 hash、compiler/runtime/Chromium/FFmpeg/font 环境、viewport、色彩、时间基、seed，以及 evaluation/output
interval。

“修改底轨能否只重渲底轨”由依赖图决定：

- 上层透明且不采样底轨时，上层中间产物可复用；
- 浏览器一次合成所有层时，底轨变化会使重叠区间的最终帧失效；
- backdrop filter、blend mode、转场或 shader 读取底轨时，相关上层节点也失效；
- 可选择分层 alpha 中间产物换更细缓存，但会增加编码、颜色和合成成本。

Onmark 支持依赖驱动的增量渲染，但不承诺“每个 shot 永远独立”。正确性优先于缓存粒度。

增量执行复用经过验证的 `FrameArtifact`，而不是散落的 PNG 或独立编码 MP4 segment。
候选 artifact 必须与该 unit 的 Browser Plan projection、实际消费的 presentation bytes、
Render Profile、capture backend 和 locked capture-environment identity 完全一致。reader
在复用前验证 payload checksum；assembly 把 PNG 交给 encoder 时会逐帧重算 canonical
raw-RGBA fingerprint。命中、新 capture 的 miss、本地执行和 worker 结果都进入同一条
artifact assembler，因此 warm execution 不会长出第二套编码或 audio path。

production authored-HTML artifact 已由 presentation contract 准入 random access，并按
Render Graph region 分别投影。Rust 通过 versioned `BundleProjection` process contract
传递每个 region 精确、有序的 shot index；bundler 验证后只做机械 DOM projection，不从
interval 重新发现 graph boundary。region document 只保留 selected shot、其 owning
scene/film shell 与已编译 motion resource，不保留其他 semantic sibling。presentation byte 按 semantic
ownership 进入 region：shot 内的 byte 只属于该 shot；scene 内、shot 外的 byte 属于该
scene 的所有 region；scene 外的 film/document byte 属于全部 region。因此 `:has()` 等
selector 无法观察 region 外的 shot，而更宽层级的 style、motion 或 resource edit 会正确
改变所有消费这些 byte 的 region。每个 region 都有独立的 `renderRegion` manifest、dense
unit-local Browser Plan node identity 和 content-derived `bundleId`。whole-film
`wholeFilm` artifact 仍用于 conformance 与低层执行，但不是 desktop partition 的 cache key。

compiler-only cue/audio element 以及素材引用和 timing attribute 会在计算 presentation
identity 前移除。修改既有 audio 或 compiler timing fact 不会使 visual artifact
失效，除非它改变 solved Browser Plan fact 或 Render Graph boundary。media byte、profile、
presentation-global byte、selected semantic subtree 与其他真实 browser input 仍全部进入
对应 identity。

完整 solved film interval 是 Browser Plan fact，因为 film/scene 级 motion 可以读取它。
只要 duration edit 改变该 interval，所有携带它的 region plan 都必须失效，即便部分 pixel
碰巧未变。Onmark 不从 JavaScript 猜测更窄的 temporal dependency；若要获得更细粒度复用，
必须先引入显式且有 conformance 证据的 capability，不能从 cache identity 偷删输入。

desktop launcher 用 pinned browser artifact、OS/architecture 与有界 system-font inventory
命名保守 host seed；native 再加入 capture mode、graphics backend 与 composition version。
显式 custom browser 不属于 launcher-owned browser identity，因此关闭 persistent reuse。
native graphics override 在取得完整 workload 的 cold-process 证据前也保持 ephemeral。
batch 只在 exact contract 已准入时使用同一份 persistent store，否则只使用 batch
lifetime 内的私有 store。cache publication 使用 deterministic capture-contract identity
寻址且原子完成；
immutable valid entry 无需全局 lease 即可读取，唯一的跨进程锁只拥有损坏修复与
publication。损坏或 identity 不符的 artifact 会被删除并重新 capture。store 同时受
artifact 数与 byte 上限约束；满额后保留既有有效 entry，让新 miss 保持 ephemeral，
不会驱逐另一个进程可能正在读取的 artifact。

每次完成的 desktop render 都会报告 region/frame 复用量；这些数字直接来自 capture 前
已经验证的 cache hit。CLI 还会报告 prepare、bundle、plan、capture、assemble 与 total
wall time。这些时间只作为运行证据，绝不进入 compilation、planning、artifact identity
或渲染结果。

仓库保留的
[incremental-rendering conformance](../../conformance/evidence/incremental-rendering.md)
验证 whole-film/partition raw-RGBA 等价、局部 edit isolation、跨 region selector isolation、
cold/warm CLI reuse、损坏修复与共享 final assembly。temporal capability 与 DOM scope 仍是
两个独立 manifest fact：random access 允许独立求值，`documentScope` 说明 artifact 实际
包含哪些 semantic DOM byte。

## 9. 当前仓库边界

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
│   ├── motion-gsap/             # 可选的 GSAP exact-frame adapter
│   ├── bundler/                 # Node/esbuild 与 bundle manifest
│   └── launcher/                # npm 安装边界与 native CLI 启动
├── scripts/                      # 仓库专用生成与质量检查
├── deploy/
│   └── aws-lambda/              # artifact conformance 后的 Rust Lambda/S3 adapter
├── schemas/
├── conformance/
├── evals/
└── docs/
```

当前仓库包含
`onmark-core`、`onmark-media`、`onmark-render`、`@onmark/runtime`
的浏览器 session、`@onmark/authoring` 的 authored-DOM bindings、`@onmark/bundler`
的 presentation artifact 边界，以及第一条 `onmark-cli` whole-film composition
root：

- `onmark-core` 是纯内核，内部用
  `syntax`、`diagnostics`、`model`、`compiler`、`timeline`、`protocol`
  模块保持结构；
- `onmark-media`
  负责素材探测、规范化 metadata 与 standalone subtitle format parsing，使服务端
  compile/lint 修正循环能够使用 `core + media` 而不链接 Chromium；
- `@onmark/runtime`
  因为运行在浏览器中、并被 authoring 与 bundler 消费而保持独立 package；
- `@onmark/authoring` 因为用户 presentation 会独立消费它的公开 DOM
  contract、而 runtime 不得向上依赖作者侧 effect 而保持独立 browser
  package；它唯一的产品依赖是 runtime 的 types-only 公开面；
- 内部 `@onmark/motion-gsap` 因为 GSAP 依赖预算独立于 vendor-free authoring
  facade 而保持单独 package；源码 workspace 中的作者使用 `onmark/motion/gsap`
  facade；根 package 的 export map 独占这条映射，bundler 只通过该表统一解析
  公开的 `onmark/*` import，不逐个选择 vendor。source-workspace mapping 本身不是
  产品合约；下述 desktop admission 才是它的 release owner；
- `@onmark/bundler`
  因为运行在 Node、独占 esbuild 与文件系统依赖预算、并产出供 native
  renderer 独立消费的 presentation directory 而保持独立 package；
- `onmark-render`
  是 Chromium、FFmpeg 编码和单机执行器的重型边界，只依赖 core-owned execution
  facts 与 render-owned materialized locations；
- `onmark-cli` 是独立发布产物，只负责参数、终端展示，以及 core compile、media
  probe、bundler process 和 native render 的组装，不把它们的实现揉进一个 crate。
- 私有 launcher package 是这份 release artifact 的 Node/npm process boundary；它只
  能依赖 Node built-ins 与锁定的 browser installer，只能启动 product bundler 和
  native CLI，其他产品 package 不得反向依赖它；
- `onmark-aws-lambda`
  是独立 Rust 发布产物：Lambda 属于不同运行环境，handler 独占
  `aws-config`、`aws-sdk-s3` 与 `lambda_runtime` 的依赖预算；它可消费
  `onmark-render` 的 portable worker request 与 `onmark-core` 的 canonical
  bundle layout，但这两个依赖都不知道 AWS。package-only 的
  `onmark-aws-lambda-package` binary 在不链接 AWS
  runtime 的前提下拥有 deterministic ZIP encoding；它是 deployment operator
  tool，不属于 repository automation，也不是 authored-video command。

### 桌面发布产物

桌面产品只暴露一个 `@onmark/cli` package、一条 `onmark` 命令，以及
`@onmark/cli/authoring` 与 `onmark/motion/gsap` facade。后者是由产品 bundler
解析的 authored-module specifier，不是第二个 npm package。内部 workspace
package 是实现模块，不是用户需要拼装的安装步骤。

私有 launcher 是很薄的 npm 边界，而不是第二套 CLI。它选择一个 optional 平台
package，并把明确的 Node、bundler、browser provisioner、FFmpeg 与 ffprobe 路径传给
native 命令。参数、诊断、编译、渲染和退出状态仍由 Rust 独占；browser provision
只会在作者诊断之后发生，ambient executable 绝不构成静默 fallback。

所有 release 自动化统一放在 `scripts/release/`：Rust module 独占 native sidecar
准入，`npm/` 独占公开 package 装配与发布，`media-toolchain/` 独占固定 media source
的获取与构建。`npm/assemble-package.mjs` 把已经构建的 TypeScript module 投影进
不超过 32 MiB 的公开 package，封闭内部 declaration import，并 hash 除自身
manifest 外的每份 payload。`cargo xtask release sidecar` 把 native `onmark`、
FFmpeg、ffprobe、source archive、build record 与 license 准入一个不超过 384 MiB
的目标 package。两个 assembler 都通过私有 staging 和最后一次 rename 发布；它们都
不编译源码、安装依赖或发布 package。

`media-toolchain/sources.json` 用 URL、字节长度和 SHA-256 固定每份 media source。
其 fetcher 是唯一的网络 owner；`media-toolchain/build.sh` 只消费已准入的本地
archive，并关闭 autodetection、network、shared library 与 nonfree component。
fetcher 对传输故障与 HTTP 408、425、429、5xx 使用四次固定、单次有界的尝试；
redirect policy、长度或 digest 违规会立即失败。sidecar 在准入目标 binary format
与 provenance 之前，会逐字节复核 source manifest 与 build script。

`packages/launcher/desktop-release.json` 是 supported target 与 browser 的唯一
contract。它独占固定的 Chrome for Testing build、browser product 与 archive
digest；native sidecar assembler 会拒绝不同的 target matrix。launcher 通过有界的
跨进程锁把所选 browser 安装进 content-addressed cache，并以 atomic rename 发布。每份
lease 都有 owner-specific heartbeat marker；被回收的旧 owner 不能发布缓存字节，也不能刷新或删除 successor 的锁。

desktop-release workflow 可以手动 dispatch 只做 admission。release input 变化时，
可信 `main` push 会运行 admission。source job 会读取受保护 squash commit 末尾的
pull-request 编号，再向 GitHub 校验该 pull request 已合并进 `main`、精确拥有当前
merge revision，且 head 为 `release/vX.Y.Z`。它不依赖 GitHub 最终一致的
commit-to-pull 关联索引。只有通过这些校验的 release owner 才能提供 proposed
product version 并启用 publication。其余 admission 与 publication job 只消费这份
revision 与 version。Linux admission 会使用组装 archive 的同一个 release
driver 校验 version。因此 admission、npm provenance 与 GitHub tag 都指向同一份已
review 的 `main` revision。两种模式都只有在空 consumer 中安装两份生成的 npm tarball，
并用两个独立 browser session 渲染同一份 screenplay 后，才准入 macOS arm64、
Linux x64 与 Windows x64。它验证精确帧数、解码音频、canonical raw-RGBA identity、
公开 product import bundling 和 no-clobber output。每个 target artifact 还会保留
固定 profile 下两次真实 CLI render duration；共享 runner 的 timing 只是 evidence
sample，不是 release threshold。cross-compilation 和 binary-format 检查本身不能
证明 target support。

release build cache 只是可丢弃的 accelerator，不是 artifact authority。media-tool
cache 由 target、已准入 source manifest 与 build script 共同寻址；cache miss 会重新
拉取并构建固定源码。Cargo cache 由 target、lockfile 与 toolchain 寻址；精确 key
变化时，可以先恢复同 target、同 toolchain 最新的 cache，再由 Cargo 逐项复核
fingerprint，并只增量重建变化后的依赖图。恢复出的 output 在发布前仍需通过相同的
manifest 校验、package assembly、空 consumer 安装与真实 render admission；冷构建与
缓存构建不会获得不同的发布路径。

普通 CI 只缓存 Cargo registry 与源码下载，不缓存 workspace `target` 目录。
workspace build graph 不是有界的 release artifact；缓存它会占满仓库 cache quota，
并淘汰 release admission 真正依赖的、更小的各平台 media-tool cache。native release
job 仍保留独立的 target-scoped Cargo cache，因为它的 producer、consumer、失效输入
与 admission check 都已在上文显式定义。

每条完成 admission 的 `main` push 都可以刷新缺失 cache。product lockfile、固定
package version、release workflow、media toolchain 或 Rust toolchain 发生变化时，
都会预热下一次 release 消费的同一条 cache path。手动 admission 也可以在所选 ref
scope 内做相同预热。publication 的正确性与可用性不依赖任一 warm path。

只有与 merged release PR 关联的受保护 `main` push 可以跨越 npm publication
boundary。这样既绕开 npm Trusted Publishing 尚不支持的 `pull_request_target`
identity，又保留 release PR 作为唯一发布决定。受保护的 `npm-release` environment
与 npm Trusted Publishing 会把该 job 绑定到 reviewed workflow，不保存长期 registry
token。其 deployment policy 只点名 `main`；该 branch 由 required PR 与 CI rule
保护。job 先验证完整且同版本的 archive 集合，再按 platform sidecar、公开 package
的顺序发布；只有 npm 返回的 integrity 与 admitted archive 完全一致，才会复用已经
存在的版本。这样多 package 发布中途失败后可以安全恢复，同时禁止同一版本获得不同
bytes。npm 接受完整集合后，同一个 job 才会在 admitted `main` revision 创建对应 tag
与 GitHub Release。publication step 会先移除 `setup-node` 注入的 placeholder
`NODE_AUTH_TOKEN`，使 npm 只能消费 job 的短效 OIDC identity。
外部 publication 失败时，只重跑同一 workflow run 的失败 publication job；GitHub 会
复用已留存的 admitted archive，不会重建任何平台。只有 admission input 发生变化时，
才重跑完整 workflow；已经存在的 npm version 仍必须与 admitted integrity 完全相同，
因此 publication retry 保持幂等。
npm 只允许为已存在的 package 配置 Trusted Publishing，因此第一个公开版本仍需
operator 使用同一组 admitted archive 做一次 bootstrap。

bootstrap 顺序固定：

1. 创建 `@onmark` organization；
2. 在目标 product revision 手动运行 admission workflow；
3. 使用带 2FA 的交互式 npm identity，先发布三份 admitted
   `@onmark/cli-*` archive，再发布 admitted `@onmark/cli` archive；
4. 让四个 package 都信任 `varo-yang/onmark` 的 `desktop-release.yml`，并限制为
   `npm-release` environment 与 `npm publish`；以及
5. 在后续合并 release pull request 前保护该 GitHub environment。

`cargo xtask release prepare <version>` 会同步 Rust workspace、每个内部 TypeScript
package 与 Cargo lockfile 的固定产品版本。改动必须放在 `release/v<version>` branch
接受 review；CI 会验证 fixed-version invariant，并要求 branch suffix 与产品版本一致。
合并这份 pull request 是唯一的自动发布决定。prerelease version 使用 `next`
distribution tag，stable version 使用 `latest`。

Lambda ZIP 仍是独立部署产物；它的 bootstrap、archive budget、`/tmp` lifecycle 与
S3 contract 不会变成桌面 installer 语义。

### 产品命令与语言证据

当前 authored native surface 刻意保持很窄：`onmark check <film.html>` 不启动
Chromium，验证到 Render Unit planning；`onmark inspect <film.html>` 解释已经求解和规划的
fact，包括精确的 video source selection 与 rate；`onmark snapshot <film.html>
--frame <index>` 把一张生产帧捕获为 lossless PNG；`onmark review <film.html>` 生成一份
精确的静态视觉审阅；`onmark render <film.html>` 执行完整 output。`onmark doctor`
验证已准入的本地工具链，`onmark info` 报告已安装产品与 host identity。machine-readable
command report 带显式版本，并保留稳定 diagnostic code 与 byte span。

authored HTML 同时包含 screenplay custom element 与 presentation DOM/CSS；至多一个带
`type="module" data-om-motion` 的 inline module 导出 declarative `motion` value，
固定 infrastructure entry 负责安装 runtime。不存在平行 stylesheet、motion 文件或 custom
entry path。CLI 只读取一次有界 HTML：Rust compiler 消费这份 owned byte，bundler 消费
同一份 byte 的私有 snapshot；相对 module import 仍从原 authored document 目录解析。因此
源码并发变化不会让 Timeline fact 与 browser DOM 来自两个 revision。若未传 `--output`，
它使用稳定且 no-clobber 的
`renders/<screenplay-stem>.mp4`。普通 render control 只有精确帧率和 viewport
dimension，process path 只是 execution override，不是 screenplay
fact。作者诊断先于 executable
preflight 输出，因此解释一份无效剧本不要求机器先装好 Chromium、Node 或 FFmpeg。Gate 三新增刻意独立的 worker
entry point：`onmark worker capture`。它只接受一份 versioned
`request.json`（包含 deployment-owned、以 SHA-256 表示的 locked
capture-environment identity）、该 manifest 列出的 `bundle/`
payload 文件和冻结的 `assets/sha256/`
字节。这个 identity 覆盖 image 中的 Chromium、字体、launch
configuration 及其他影响像素的 host facts；renderer 刻意不从单一 executable
path 或 browser-version
string 猜一个不完整的身份。worker 在私有 root 中 materialize 后发布一份 frame
artifact，reuse 与 assembly 都要求 environment identity 和 unit
identity 同时匹配。它不接受 screenplay、绝不重新编译 source；短生命周期 invocation
owner 或 object-store adapter 负责发布 request。

CLI 在 core parse 之前使用带 sentinel 的 8 MiB 上限 UTF-8 reader 读取剧本，core
还会独立执行 syntax byte、retained-item 与 nesting 上界。worker capture
属于另一个信任域，对 request JSON 使用 16 MiB 上限；这个值由 render
单点拥有，并由本地命令与 deployment adapter 共用。两个入口都不会使用可能按不可信文件长度分配内存的整文件 convenience read。

`onmark-cli`
在启动外部工作前一次性解析全部 executable，然后按线性路径执行：read/compile →
freeze/probe referenced assets → solve Timeline IR → bundle presentation →
compose/materialize whole-film unit → render → atomic
publish。冻结过程一边把每个引用源流式复制进私有临时文件一边计算 SHA-256，之后只 probe 这份私有副本，因此 identity 与 metadata 对应同一份 retained
bytes。hash/probe 在显式 blocking work 中执行，不占用 Tokio
worker。CLI 以 core、media、render 为真实 composition input；`clap`
只负责参数解析，`sha2` 只负责流式 SHA-256，`tempfile`
只负责私有生命周期，`serde_json` 只解码 Rust-owned
manifest，Tokio 只负责有界 process/render async
work。这些依赖都不能进入纯 core。

`evals/` 是 checked-in 的语言产品证据，不是 runtime
package，也不是 CI 中调用在线模型的服务。它拥有冻结的题目、prompt、grader 规则、原始输出、模型参数和对照 baseline。只有真实实验材料可用时才加入这些资产；仓库不创建空框架，也不凭记忆伪造历史 baseline。仓库自动化可以在无网络时解析并重新评分冻结输出，但绝不把 CI 变成在线模型 benchmark。

`onmark-media` 必须独立而不能藏在 render
feature 中，因为“无 Chromium 的素材探测服务”是明确消费者，同时满足依赖预算和独立消费两条判据。Feature 只表达同一包内正交能力，不能用来遮住真实存在的架构边界。

Render Graph 和 planner 在第二关先作为 `onmark-core`
模块加入。只有出现独立消费者、编译成本或清晰发布边界后才考虑拆 crate。worker 状态机属于
`onmark-render`；远程编排保持为外部短生命周期 composition concern，除非未来产品证明必须拥有持久协调。

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

`syntax` 不得依赖 compiler，`timeline`
不得依赖 syntax，领域模块不得反向依赖 protocol。CI 使用 `syn` 对显式 Rust
path 做语法感知检查。这是一条协作式护栏，覆盖普通路径、import、alias 和 re-export，但不覆盖宏内部生成的路径，也不等价于 rustc 的完整名字解析；这些边仍由评审负责。任何新增内部边必须先更新本文。

`onmark-core` 只允许 `syntax` 使用 `html5gum` 做纯计算、保留 span 的 HTML
tokenization。严格 authored tree 构建、嵌套检查、重复属性检查、资源上界和全部创作语义由
Onmark 自己拥有；browser recovery 不决定 screenplay ownership。tokenizer error 在 syntax
边界翻译，该依赖不执行 IO。`@onmark/bundler` 只用 `parse5` 定位唯一获准的 inline motion
module，并保留其他全部 authored bytes。测试 target 可以使用 `proptest`
验证时间代数，并使用 `syn`
执行协作式模块依赖律检查；二者都不会链接进库消费者或运行时产物。

`@onmark/runtime` 继续保持 vendor-free，并独占精确 frame effect 与 resource lifecycle；
`@onmark/authoring` 拥有 authored-DOM binding 和 vendor-neutral 的
`PresentationExtension` contract。
它的 `/types` subpath 只导出 declaration，optional adapter 不能沿这条依赖边取得
authoring runtime behavior。内部 `@onmark/motion-gsap` 具有不同的第三方依赖预算，并承载
workspace 内的 `onmark/motion/gsap` facade。只有它依赖固定版本的 GSAP，把 Rust-owned
interval 投影成局部 browser seconds，seek 时抑制 callback dispatch，并在 terminal
disposal 时 kill 每条 playhead。其他引擎实现同一 extension contract。它只允许依赖
`@onmark/authoring` 与 GSAP，消费者是 authored motion module；
bundler 与 runtime 都不选择 vendor。Three.js 在出现同样窄且通过准入的 production
adapter 前，仍只是 repository development dependency。

browser projection 从 Timeline IR 保留 film、scene、shot 与 content ownership。node
identity 在每个 Browser Plan 内 dense 且 canonical；authored ID 保留跨 projection 的语义
身份。每个 node 还携带适用的 solved interval。video 与 authored overlay 指向 owning
shot；导入 caption 保持 film-level。wire 继续使用 flat relational plan，让 native
validation 与 region projection 保持有界；authoring adapter 把每份 plan 绑定到对应的
whole-film 或 region DOM。TypeScript 不得重新求时或推导分片。

`protocol` 模块使用 `serde` 定义稳定的 browser 与 bundle-manifest
JSON 边界。其可选的 `schema` feature 只为仓库生成工作暴露
`schemars`，产品 binary 不启用它。所有仓库专用自动化统一放在
`scripts/`；它既不是产品 package，也不是杂项应用层。其中的 Cargo
manifest 只用于给 Rust schema generator 一份固定的 build-only 依赖预算和稳定的
`cargo xtask` 入口。这个 binary 只由开发者与 CI 消费，可以依赖启用 `schema`
feature 的 core 与 `onmark-aws-lambda`、`schemars`、`serde`/`serde_json`、只负责固定
产品版本的 `semver`，以及只负责 native release identity 的 `sha2` 和只负责私有
sidecar staging 的 `tempfile`；它会
关闭 Lambda package 的默认 runtime feature，因此 schema generation 不链接
AWS。任何产品 crate/package 都不得反向依赖它。Lambda 依赖只为发布该部署边界的
schema，不得借此把 AWS 偷渡进 core。相邻的 Node generator 可使用固定版本的
schema-to-TypeScript 与验证工具链。`cargo xtask schema` 先写全部 versioned schema，
再调用该 generator；`cargo xtask eval audio`、`audio-envelope`、`html`、`transition`
与 `video` 会在不调用 live model 的前提下分别重新评分已冻结的 language experiment。
`cargo xtask release prepare/verify` 独占产品版本修改与一致性检查；
`cargo xtask release sidecar` 只装配 native platform payload。相邻 release scripts
负责装配并准入公开 npm package 与媒体；只有受保护的 release workflow 会调用 registry
publisher；
`scripts/` 内部由 `evaluation/` 独占 frozen language grading，`schema/` 独占
Rust-to-TypeScript contract generation，`typescript/` 独占全仓库 source-shape
mechanics，`release/` 独占 desktop artifact construction 与 publication；根
`xtask.rs` 只负责把命令分发给这些 owner。release owner 内部再由 `npm/` 与
`media-toolchain/` 把 Node 和 shell process boundary 与 Rust xtask module 分开。
`json-schema-to-typescript`
把 browser 类型生成进 runtime、把 manifest 类型生成进 bundler，Ajv 在构建期生成 standalone
browser validator。Lambda schema 在出现真实 TypeScript caller 前刻意不生成
codec。Oxlint、窄范围 repository-shape check 与 Prettier 都只属于仓库开发工具，绝不
进入产品 artifact。real-process CI 使用固定版本的 `@puppeteer/browsers` 下载测试所
声明的精确 Chrome for Testing headless-shell build；桌面 launcher 则把同一 library
作为 production dependency，用来验证并展开已准入的 release archive。浏览器 runtime
不在运行期动态编译 schema。精确工具版本由 lockfile 与 `mise.toml` 固定，CI 会拒绝
过期生成物。

### 媒体归一化边界

`onmark-media` 只依赖 core，以及用于私有 ffprobe response 边界的
`serde`/`serde_json`。它使用参数数组直接启动配置的 ffprobe
executable，绝不经过 shell；退出后仍让派生进程持有输出 pipe 的 wrapper 不属于该 executable
contract。在这条 direct-child 契约下，进程寿命和保留的 stdout/stderr 字节数都有显式上限，两条 pipe 并发排空；显式 shutdown 会报告 process-control
failure，`Drop` 只作 best-effort termination fallback。私有 ffprobe response
type 只在此边界翻译一次并产出 core-owned `AssetMetadata`；JSON
value 与第三方 error type 不定义稳定 API，但底层 error 会通过标准 source
chain 保留，供调试使用。探测对每条 stream 请求有界的 stream-level
facts。attached-picture video stream 不属于可渲染媒体；其余 video stream 与 audio
stream 分别优先选择声明为 default 的流，default 缺失或并列时按 ffprobe 报告的最低
stream index 确定。`sample_rate` 与 `channels` 固定 selected audio stream 的 sample
grid 与归一化 channel layout。第二次 probe 只读取 selected visual stream
的完整 best-effort timestamp 序列、精确 media timebase 与终止 timestamp。单帧即
still；多个相等 timestamp interval 证明精确有理 constant frame rate；不等 interval
形成完整 `VideoFrameMap`。nominal stream rate 不能代替这份证据。visual duration
来自归一化后的终止边界，不再来自十进制 stream duration。固定的十六 MiB
stdout/stderr ceiling 足以承载 browser contract 的 100,000 个边界，同时让两条 process
pipe 保持有界。

Gate 四同时把 standalone-subtitle syntax boundary 放进 `onmark-media`。parser 在显式
input bytes、cue count、单 cue text 与固定 retained-error 上限内消费 caller-owned bytes，并返回
带 source location 的 format error，或带精确纳秒 interval 的 core-owned `CaptionTrack` fact。该边界
不访问文件系统、不猜编码、不解释样式、不翻译 diagnostic code，也不决定 browser layout。首批格式是
strict UTF-8 SubRip、无损的 plain-text WebVTT 子集和无损的 plain-event ASS 子集，三者都支持可选
UTF-8 BOM 以及 LF/CRLF 换行。WebVTT comment 与 cue identifier 不携带 rendered fact，可以丢弃；
region、style block、cue setting、markup 与 escape 则必须报告 unsupported。Plain ASS 接纳
`ScriptType: v4.00+`、安全 script metadata 与 `Format: Start, End, Text`，精确换算 centisecond
时间并处理 `\N` 和 `\h`；resolution、style、layout field、effect、override tag、drawing 与其他呈现
语义都必须明确报告 unsupported。三种格式共用同一个 fact boundary，且不给 crate 增加 production
dependency。
CLI 根据 authored file extension 选择唯一 parser，并在 presentation validation、media probe
或 browser launch 之前，把 format-local error 恰好一次翻译成 `ONM-CAPTION-*`
diagnostic；文件打开与读取失败仍是 typed infrastructure error。

### 浏览器与编码器边界

`onmark-render` 拥有 Chromium 与 `FFmpeg` 的重型依赖预算。它只把 `chromiumoxide`
用作 CDP transport。Onmark 自己启动并回收 headless shell，使 stderr 在
`DevTools` endpoint 出现后仍被持续排空到有界 diagnostic tail。 `base64`
只解码 CDP 规定的 screenshot envelope，`png` 只用于把 capture screenshot
decode 成 renderer-owned canonical RGBA fingerprint；`tokio` 和 `futures`
也只存在于这条异步执行边界。`tempfile` 为每个 browser
session 提供隔离 profile、为每个 output 创建同文件系统的私有暂存目录，并为每个
executable unit 保有一个 RAII 私有 resource root。

unit-root materialization 只用 `serde_json` 编码 Rust-owned manifest、用 `sha2`
流式复核 identity、用 `url` 构造 browser entry URL。file
bound 会在 identity 工作前拒绝，canonical hash 与 manifest
size 都通过固定内存 writer 流式计算，pretty
manifest 直接写入私有 root。它拒绝 symlink 与非普通文件，复制已验证字节而不链接可变 source
path，同时限制保留文件数与总字节。固定 safety
envelope 是十万个文件与一 TiB，每个调用方仍须提供更小的显式 policy。因此并行 sequence 既不共享 Chrome
lock，也不共享已接纳的输入路径；只有 Chromium 与 `FFmpeg`
都干净结束后，才用 no-clobber hard link 发布完整 MP4。

crate 显式提供 executable path、viewport、browser process/request
deadline、encoder inactivity timeout、frame/input/capture byte
ceiling、有界 stderr 保留与 shutdown，并把 Chromium、CDP、subprocess 类型翻译成 render 自己拥有的稳定错误。有限 frame/byte
budget 与每次 write、finalization 的 timeout 共同约束 encoder 生命周期。video encoder 的
精确 thread count 同样属于显式 host policy：local CLI 默认使用四线程并接受有界显式覆盖，
portable worker 保持单线程，两条路径都不从 ambient CPU count 推导。等待 Chromium 的时间
不消耗 encoder inactivity budget。浏览器导航会分别等待 document load 与 runtime
host；不能把 transport 的 navigation 返回误当成完整生命周期屏障。

browser capture 最多只保留一张 PNG，捕获后直接写入 `FFmpeg image2pipe`。分布式
layered capture 因 frame artifact 需要拥有 canonical RGBA 与 hash，使用一条 capacity-one
stream 返回像素；本地 layered video 只把透明前景送入 encoder process，不会 split 已合成帧，
也不会在写入所选视频 profile 前把 raw RGBA 再复制回 Rust。不存在整段视频 frame buffer。
共享的 subsampled 输出 profile 会在进程启动前拒绝奇数 viewport 尺寸。browser capture 在共享 runtime
protocol 下只有两个封闭 backend：Linux `chrome-headless-shell` 用 `BeginFrame`
原子提交并读取 compositor transaction；macOS 与 Windows 用 `Screenshot`，在同一个
`Seek` readiness barrier 后通过 `Page.captureScreenshot` 读取 surface，并复用同一条
post-capture `Confirm` 与 placement-boundary reconciliation。portable backend
不会引入第二套 clock、timing solver、plan、encoder 或 media-selection path。所选
backend 会显式报告并进入 capture-environment identity；只有等价的 locked environment
与 backend 之间才断言相等。

capture capability 属于经过准入的 browser artifact，而不是它的文件名。managed Linux
browser package 显式声明 `BeginFrame`，managed desktop package 显式声明
`Screenshot`。通过 `--browser` 传入的任意路径，即使 basename 看起来像 headless
shell，也只获得 portable screenshot contract。底层 executor 显式接收 mode，因此
symlink 或重命名 binary 不会悄悄改变 compositor protocol。

conformance 会启动固定版本的 Chrome for Testing browser 与 `FFmpeg`，加载 production presentation
adapter，走过类型化 `Load`/`Prepare`/`Seek`/`Confirm` 握手，probe 最终 H.264
MP4 并验证 decoded motion。checked-in bundle fixture 携带真实 payload
bytes，由 bundler test 逐字节重建，并通过 native
materialization 穿过生成的 Node/native manifest contract。最外层 CLI
conformance 会启动两次独立的 whole-film session，分别验证 decoded
output 的帧数、运动、stream facts 与 audio
placement，再验证 no-clobber 发布。canonical raw-RGBA 相等性仍由 native capture
boundary 断言；独立编码的有损 MP4 帧不是 identity
oracle。CI 显式拥有这些测试使用的 browser 与 media-tool 版本：Linux 锁定 canonical
BeginFrame path，desktop release admission 在 macOS 与 Windows 锁定 portable screenshot
path。

GitHub-hosted Ubuntu 会把 AppArmor user-namespace 限制施加到下载的 Chrome for
Testing binary。desktop release admission 会安装一份 runner-local AppArmor
profile，只向 content-addressed Onmark browser cache 路径授予 `userns`，从而保留
Chromium 自身的 sandbox。更底层的 real-process suite 仍使用一次性的 runner-local
`--no-sandbox` wrapper；两种 CI 例外都不会改变产品 launch policy。产品与本地
browser launch 默认仍然启用 Chromium sandbox。canonical default 与所有 worker
policy 都显式锁定 ANGLE 的 `SwiftShader` backend：host GPU 的可用性不能悄悄改变像素，
也不能使 whole-film 与 partition capture 产生分歧。browser process 还会关闭 GPU
rasterization、partial raster reuse 与 runtime-selected Skia optimization，锁定 sRGB，
并在 readback 前排空所有 compositor stage。这些 switch 合起来是一份 exact raster
contract，而不是互相独立的性能旋钮；它们属于 code-owned capture-environment
composition version。

macOS desktop execution 仍可显式选择 `Metal` graphics backend，并在 page execution
前通过 CDP 反查实际 GL renderer；Chromium 若回退到 software renderer 就直接失败。
这是独立 capture environment，而不是 `SwiftShader` identity 的更快实现：
backend-sensitive WebGL pixel 预期会不同。opt-in macOS conformance 覆盖独立 Metal
session、重复与乱序 seek、WAAPI、GSAP 和 Three.js effect。在完整 mixed workload
取得与 canonical software contract 相同的 cold-process 证据前，Metal 只作为显式且
ephemeral-cache 的 override。其他 native backend 同样必须独立准入。

首轮锁定 macOS 性能测量使用 Apple M5、Chrome for Testing 149.0.7827.55 与 release
CLI，把 checked-in CSS/GSAP presentation 以 1,920×1,080 渲染 45 帧。三次独立
`SwiftShader` 用时 10.80、6.61、6.56 秒，三次 Metal 用时 7.28、4.66、4.73 秒；warm
pair 约快 29%。这些是包含 compile、bundle、browser launch、capture 与 encode 的端到端
CLI 样本，不支持扩大为跨平台结论；它们早于 exact raster contract，只保留为历史
native-backend 证据，不再描述当前默认 policy。

exact-raster follow-up 在同一 host 上使用 Chrome for Testing 149.0.7827.55。应用该
raster contract 前，435 帧、7 个 region 的 mixed-media campaign 在两个 cold capture
间有 430 帧 raw-RGBA 不同；应用后，两份独立 cache directory 与 browser process 产出的
435 帧全部精确相等，capture 分别用时 256.70 与 306.37 秒，仍在此前观察到的 software
区间内。一份更小的 75 帧、5-region CSS/GSAP fixture 也逐帧相等，capture 分别为 32.63
与 33.16 秒，略低于 partial-raster control 的 34.68 秒。这组数据准入 pinned
`SwiftShader` contract 的 persistent reuse，但不声称 CPU raster 在所有 workload 上更快。

本地 CLI 默认给最终 H.264 encoder 分配四个线程。CPU 或内存预算不同的 host 可以通过
`--video-encoder-threads` 显式选择 1 到 64；Onmark 不从 ambient core count 推导这个值，
否则编码资源与输出字节会在不同 host 间悄悄变化。portable capture worker 保持
deployment-owned 的单线程策略并通过 partition 横向扩展；worker 本身不编码最终组装的
MP4。

一个经过校验的本地 partition sequence 只保留一个 Chromium process 与一个连续
encoder。每个 unit 仍有独立的 runtime navigation、类型化
`Load`/`Prepare`/`Dispose` 生命周期、私有 resource policy 与清空的 screenshot
cache。前一条 resource guard 必须先退役，下一条 root policy 才能安装，因此 process
reuse 既不会扩大文件访问范围，也不会跨 unit 复用 no-damage frame；worker artifact
仍保持每个 unit 一个 browser process。Apple M5 上一组 640×360、30 fps、每个 semantic-DOM
shot 为 100 ms 的探索性样本暴露了固定成本：四个 unit 从 2.49 秒降至 1.29 秒，八个从
5.22 秒降至 2.18 秒。real-process conformance 继续要求 whole-film 与 partitioned
decoded-frame hash 相等；这些样本用于确定生命周期边界，不是 release performance threshold。

同一 sequence 只有在 bundle 声明 `placementBounded`，且已准入 unit 证明 Chromium
不拥有 video pixels 时，才可以复用 browser foreground capture。复用会在每个 plan-owned
placement boundary 停止，也绝不跨越 unit navigation。local render 与 worker capture
消费同一个 serialized cadence，任何一条路径都不另造 cache policy。
real-process layered fixture 会把这条路径与除 frame behavior 外完全相同的
`everyFrame` bundle 对照：control 的 75 个 output frame 都会进入 browser capture，
placement-bounded candidate 只有一个 frame 会进入，而两个独立进程产出的 canonical
raw-RGBA sequence 仍逐帧完全相等。有界 retry 与 reconciliation readback 继续进入 phase
timing，不伪装成额外 authored frame。

一组 Apple M5 release real-process run 使用 Google Chrome 150.0.7871.186、
`Screenshot` backend、`SwiftShader` 与 75 帧的 1,920×1,080 layered fixture，把 authored
capture 工作与实际 Chromium command 分开计量。`everyFrame` control 进入 capture 75 次，
实际发出 76 个 pixel command；readback、pixel processing 与 native write 分别耗时
3.63 秒、6.7 毫秒与 298 毫秒。`placementBounded` candidate 只进入 capture 一次并发出
两个 command；相同阶段分别为 367 毫秒、0.07 毫秒与 169 毫秒。两条 canonical raw-RGBA
sequence 逐帧完全相等。两次运行约一秒的本地 browser launch 不是 Lambda 证据：当前
cadence 尚未部署，历史 Lambda launch 样本使用的是不同代码与 binary。

随后一组 Apple M5 encoder isolation run 用相同的 45 帧、1,920×1,080 layered input
比较 x264 的一、二、四、八线程：wall-time median 分别为 1.08、0.84、0.68、0.63 秒，
observed process peak RSS 分别约为 533–541、545、561–577、605–615 MiB。四线程保留了
绝大多数加速，同时避开最后一级的内存成本。完整 release CLI 在固定四线程 policy 下 warm
重复运行得到 4.17 与 3.99 秒，MP4 bytes 完全一致。改变 thread policy 本身不能作为 identity
比较：x264 是有损编码，同一 canonical input 也可能产生细微不同的 decoded pixels，因此
确定性 visual oracle 仍是 raw RGBA。

direct screenshot encoder 与 layered media encoder 共同消费同一份封闭输出 policy。
交付 profile 使用 x264 `medium`、CRF 18、`yuv420p` 与 BT.709 limited-range metadata；
剪辑 profile 使用 `prores_ks` profile 4、`yuva444p10le`、16-bit alpha plane 与相同
color declaration。
这些事实必须显式声明，不能继承 FFmpeg default；FFmpeg 升级不得暗中选择 codec、
pixel format、alpha policy 或 container。

本地 capture 保留 Chromium 的标准 multi-process
topology；只有独立审计过的外层 container 或 microVM 承担等价的 process
isolation 时，adapter 才能选择
`BrowserLaunchPolicy::isolated_worker()`。该 policy 同时使用 single-process、no-zygote 与 in-process
SwiftShader，而不是禁用 graphics stack。这个部署拥有的选择必须属于 locked
capture environment，不能由 authored input 或 worker
invocation 选择，并且必须在真实执行环境中证明后才能成为 production launch
contract；Chromium launch 失败绝不触发自动降级。

native browser operation 与 decoded-video
wait 最多接受一天 deadline，使所有平台 timer 都处于显式支持的时间范围内。

校验失败原因保留为局部领域值。syntax 提供源码上下文后，由 `compiler`
模块唯一负责把 `InvalidNodeId` 等原因翻译成带源码位置的
`Diagnostic`，包括各阶段特有的 message 和 help；`diagnostics`
只拥有通用诊断表示与稳定 code。`model` 和 `syntax`
都不依赖 diagnostics，调用方也不得重复实现这层翻译。

### TypeScript package 方向

```text
@onmark/runtime ←── @onmark/authoring ←── @onmark/motion-gsap
       ↑                    ↑
       └──── @onmark/bundler ┘
```

`runtime`
是浏览器底座和长期稳定扩展点，拥有当前帧 hook、FrameReady 协议和 adapter
contract。`stateless/warmup/sequential` 目前只是架构分类，不是公开 capability
declaration；该 API 成为现实后也只能由 runtime 拥有。`authoring` 的 package root
只通过 runtime 的 types-only entrypoint 使用公开类型，创建语义化 video/overlay
DOM，并把 CSS 与 layout 留给 presentation。bundler 生成的 neutral entry 把 runtime
value 组合到这些 binding 周围。`bundler` 注入固定 authoring/runtime artifact 并生成
manifest；runtime 永不依赖 authoring 或 bundler。
`RuntimeSession` 拥有 protocol 顺序、interval 关系检查、精确帧投影与 terminal
disposal；并发 command 直接拒绝，不暗中增长队列，adapter 只会收到递归冻结的 plan
snapshot。浏览器具体工作只通过一个窄 adapter 进入，其等待必须有界，预期失败必须类型化。production
presentation adapter 接收 presentation-owned element、source 与 visibility
effect；它负责有界媒体加载、精确 source-frame selection、decoded-frame
readiness、已求解 overlay visibility 与 terminal
cleanup，但不创建 layout 或 canvas state。Gate 六在同一 owner 下加入封闭的
`image | font | texture | custom` resource boundary。每个 presentation 最多保留 256 个具有唯一 identity
的 resource；`Prepare` 在同一共享 readiness policy 下并发启动它们，报告全部超时的
`kind:id:prepare`，terminal cleanup 则按声明顺序等待全部 resource。adapter 与 bundler 使用的 materialized
asset directory 同样由 Rust bundle schema 生成。

`@onmark/bundler` 是 Node-only 的产品构建边界，不是仓库自动化。它只允许依赖 Node
built-in、`@onmark/authoring`/`@onmark/runtime` 的公开入口和固定版本的生产依赖
`esbuild`；浏览器 package 不得反向依赖它。bundler 只编译单个 ESM
presentation、替换为固定 authoring/runtime 入口、生成固定 document
shell，并以稳定 SHA-256 manifest 记录每个 presentation
payload 文件。package 通过窄 `onmark-bundle` executable 暴露同一个操作，native
CLI 因而不 import Node 或 esbuild type。child
process 只接收显式 entry、output 和 retained-byte-limit 参数，成功时不向 stdout 写 payload，失败时向 stderr 写稳定类别；native
caller 自己施加 process
deadline，持续排空诊断但只保留有界 tail，并把产出的 manifest 重新交给 Rust-owned
wire type 解析。manifest shape 与 layout constants 都来自 Rust protocol
contract 的生成结果，不在 TypeScript 手写第二份。构建显式限制最终保留字节数，经输出目录同级的私有 staging
directory 写入，并拒绝构建前或发布前已存在的输出路径。最后一次 directory
rename 能防止读者看到正常完成过程中的半成品，但 Node 的可移植文件系统 API 无法把此前的 absent
check 变成跨进程 no-clobber transaction。Gate 六首个 resource slice 为本地 AVIF、GIF、JPEG、PNG、
SVG、WebP、OTF、TTF、WOFF 与 WOFF2 import 配置一份封闭集合。Esbuild 把 module 与 CSS import
写到不透明的 `resources/<hash>.<extension>` 路径。bundler 也会冻结原生 HTML `img[src]` bytes，
把引用改写成 SHA-256 resource path，并让每个 projected shot document 只保留自身引用的 image。
browser adapter 会自动把这些原生 image 纳入有界 decode-readiness 生命周期。bundler 用一份共享的
byte-level admission 拒绝会由 browser wall time 自行推进的 image container 或 SVG 能力；本地
`src`、data URL 与 generated image import 共用这条规则。随后 bundler 归一 generated reference，
由既有 manifest 独占 canonical SHA-256 与 retained-byte bound。本步骤不做 image/font decode，
也不构成 browser-readiness 证据；它只识别拒绝 ambient playhead 所需的封闭 container 与 markup
特征，不转换 image bytes。当前边界刻意不提供 watch、plugin API、cache、development server、
external fetch 或通用 asset transformation policy。
Esbuild 内部工作内存仍由固定的第三方实现管理，不受 retained-output ceiling 约束。

`@onmark/launcher` 是公开桌面 artifact 内部的 Node/npm boundary。它只允许依赖
Node built-ins，以及固定版本的 browser download、proxy 与 ZIP extraction
libraries。它选择一份已准入的平台 sidecar，把明确的 product-tool path 传入 native
CLI，并独占 verified browser cache；它不得 import bundler、compiler、timing、
render graph、browser runtime 或 authoring semantics。它的消费者只有生成的公开
npm package 与 release conformance。

### AWS Lambda 是适配器，不是第二套引擎

第三关当前引入独立 Rust binary
`onmark-aws-lambda`，因为 Lambda 是不同运行环境，且 handler 拥有独立的 AWS
SDK 依赖预算。其 deployment-only browser boundary 另外使用 `sha2`、`tar` 与
`zstd` 校验并展开一个有界 immutable payload；archive
type 与 policy 在 adapter 截止，`onmark-render` 只接收 executable
path，并为 Chromium child 发现可选的相邻 runtime sidecar。它当前拥有 V1
invocation/result schema、thin handler 与 S3 operation：

```text
decode invocation
→ bounded download of portable worker input
→ materialize Render Unit through onmark-render
→ capture and verify immutable artifact
→ conditional S3 publication
→ return structured result
```

完成 multipart upload 时，`If-None-Match: *` 的 `412`
表示“下载、完整验证，并对比已发布 artifact 的 raw-RGBA”；有界的 `409`
retry 仅是 conditional-publication transport retry，不是分布式 retry
policy。这个 JSON contract 已由 Rust 生成 checked-in
schema。它刻意不生成 TypeScript AWS SDK：当前没有 TypeScript
caller，为了对称而先造 remote orchestration client 只会发明不存在的 consumer。

部署配置单点拥有 S3 transport budget：connection timeout 为五秒、单次 attempt
timeout 为四十五秒、完整 operation
timeout 为九十秒，SDK 最多尝试三次。`GetObject` 在 SDK
operation 完成后才交出 response stream，因此每一次仍在等待的 body
read 另有三十秒 progress
deadline。这样能拒绝卡住的 stream，但不把这条 transport 边界伪装成 scheduler 或 lease
policy。
execution role 只能在配置的 artifact prefix 下条件删除 invalid artifact。repair 使用
失败读取返回的 ETag 约束 `DeleteObject`；若 precondition race，worker 会做有界重读，
不会删除另一个 worker 已经写入的 replacement。

部署提供 already-expanded headless shell，或者一份 zstd-compressed tar
archive 加 canonical SHA-256 digest。archive materialization 同时限制 compressed
bytes、expanded bytes 与 entry count，并拒绝路径穿越、重复路径、link、special
file、digest drift 和不可执行的 shell。可选字体会得到私有 fontconfig
file 与 cache；renderer 只向 Chromium child 传递该配置、相邻 shared library
path 与 SwiftShader manifest，不修改 process-global environment。browser
preparation 在每个 Lambda execution
environment 中 lazy 且只执行一次，因此 Runtime API 会先启动，warm
invocation 直接复用已校验的私有 installation。

package-only 的 `onmark-aws-lambda-package` binary 消费一个预构建的
`provided.al2023.arm64` bootstrap、一份 self-contained Linux arm64 `FFmpeg`
executable 与 expanded browser root。它排序 portable
browser path，拒绝 link 与 special file，归一化 tar
ownership、mode 与 timestamp，固定 single-threaded zstd policy，并固定 ZIP
order、timestamp、permission 和 compression level。通过 sibling
staging 发布的 output directory 只包含 ZIP 与 canonical manifest；最终 directory
rename 能避免正常完成时暴露半成品，但 portable filesystem
API 无法把此前的 absent check 变成跨进程 no-clobber
transaction。manifest 记录 bootstrap、browser archive、`FFmpeg` 和最终 package 的 SHA-256
identity。capture-environment identity 保守覆盖 bootstrap、browser
archive、`FFmpeg`、target 与 isolated-worker launch policy；bootstrap digest 同时拥有编译进
`onmark-render` 的 native composition policy。这保证“相同 locked
inputs 得到相同 outputs”；cross-compilation 仍属于 pinned Linux arm64
builder（例如 Cargo Lambda），不由 packager 假装完成。packaging 会拒绝非 Linux
arm64 executable，并在 Lambda 250 MiB unzipped-package
ceiling 下预留十 MiB 余量。

它不复制 compiler、frame handshake、FFmpeg
plan 或 cache-key 逻辑。AWS 与 browser-archive 类型不允许进入 `onmark-core` 或
`onmark-render`。上述真实 arm64 Lambda experiment 已为一份 locked 30-frame
title-only fixture 证明 outer isolation、constrained-process BeginFrame
capture 与 immutable reuse，并用 fresh container
layer 对照确定性能问题来自 browser delivery 与 preparation phase，而不是 Rust
handler 或 BeginFrame 本身。`deploy/aws-lambda` 现在能从 locked
inputs 复现 reviewed ZIP 与 manifest；infrastructure
definition、cross-build 与 Lambda package publication 仍需独立 review。

如果将来出现 GCP、ECS 或 Kubernetes backend，它们只是同一执行器的另一个 deploy
adapter，而不是新 renderer。它们各自拥有 SDK、transport semantics 与 release
artifact；Lambda environment variable、ZIP layout 和 S3
policy 不会被抽成伪通用 cloud interface。

### Schema 的单向来源

需要区分两类 TypeScript 类型：

- 已公开的 browser、bundle 与 worker message 属于跨进程 wire protocol；
- components、resources、effects 属于手写的 authoring API。

Timeline IR 与 Partition Plan 目前只有结构确定的 Rust 内存表示；在出现真实外部消费者前，不提前冻结公开编码。

Rust wire types 是 source of truth。`cargo xtask schema`
从它们生成 checked-in、versioned JSON
Schema，CI 重新生成并要求工作树零 diff。存在真实 TypeScript
consumer 的 schema 同时生成 checked-in types/codecs；目前 browser 与 bundle
contract 属于这一类，Lambda invocation 在没有真实 TypeScript
caller 前刻意不造 codec。生成结果提交进仓库，供 npm package、diff
review 和非 Rust 消费者直接使用；禁止手工修改。browser 与 worker contract
是 build-coupled internal artifact：不兼容变化必须递增当前 protocol version，
替换 checked-in schema 与 example，并重建依赖它的 bundle 和 worker request。
consumer 只接受当前版本，不保留 legacy decoder；released product version
就是迁移边界。Rust 本身直接使用原始领域/wire types，不再从 schema 反向生成
第二套 Rust 类型。

`BrowserPlan` 现在携带 production presentation adapter 已真实消费的 output frame
rate、完整 solved film interval、evaluation/output interval、film/scene/shot structure、
primary-video placement，以及 title、call-to-action 或导入 caption overlay。与 unit
相交的 structure 和 overlay 保留完整 solved interval；evaluation 只定义执行窗口，output
只定义发布窗口，两者都不能改写 presentation time。每个投影 node 都记录 dense
unit-local compiler-owned identity 与可选 authored identity；跨 projection 的语义身份由
authored identity 承担，content 显式指向其 structural parent。video placement 另记录
immutable asset identity、完整 CFR 或 VFR source timing、所选 source interval、冻结素材的
natural end，以及验证 decoded-frame selection 所需的 canonical playback、重复次数与
final-frame hold；overlay
placement 记录封闭的语义角色与 decoded text。materialized URL 仍是 render-owned
fact，DOM 结构与 CSS 则始终是 presentation-owned effect。这是一条 Render
Unit 的 browser-facing projection，不是 Render Graph 或 partition
plan 本身。它只能包含浏览器真实消费的事实；output path、cache
key、FFmpeg 参数、source span 和 materialization policy 都不得进入。更多 component
事实等 production adapter 真正消费时再加入，不提前把后续 gate 塞进协议。

一份 Browser Plan 最多分别携带 10,000 个 scene container、shot container、video placement
与 overlay placement；每条 overlay
inscription 最多包含 65,536 个 Unicode 字符。native projection 与 Rust wire decode
还会在 CDP serialization 前，把每份 browser plan 的合计 UTF-8 text 限制为一 MiB；该 aggregate
process budget 不会被伪装成 JSON Schema 能表达的结构约束。一条 failure 最多包含 4,096 个 message 字符与 256 条 pending-resource
description，每条 description 最多 1,024 个字符；它们的确定性顺序由 producer 拥有。runtime-host
property name 与这些 resource limit 都从 Rust-owned schema metadata 生成，native
executor、browser runtime 与 validator 不得各自保存手写副本。

authoring API 可以追求浏览器端人体工程学，但不能复制求时语义。

```text
Rust wire types → checked-in versioned schema → generated TypeScript codecs

handwritten TypeScript authoring API → screenplay source → Rust compiler
```

## 10. 产品表面与可观测性

Gate 一唯一承诺的命令是：

```text
onmark render film.html -o film.mp4
```

`check`、`compile`、`inspect`
属于后续可能从真实使用中长出的产品表面；Gate 三当前唯一的 `worker`
表面是同一执行器的窄部署适配器
`worker capture`，不是 coordinator。其余命令都不是当前 CLI 合约，也不能提前生成空命令或 coordinator 脚手架。

Rust API 用于嵌入服务端；TS API 用于 authoring；跨进程使用 versioned
schema，不直接暴露内部领域对象。CLI 输出、诊断码和 Execution
Plan 都是稳定产品协议。

每次 render 有 render ID，每个 unit 有 attempt
ID。Trace 贯穿 compile、bundle、schedule、prepare、capture、encode、upload 和 assemble。核心指标包括单帧 capture/encode 时间、CPU/RSS、channel 深度、缓存命中、重试阶段、网络字节、临时盘峰值和 planner 估算误差。

## 11. 安全边界

用户 HTML/JS 是不可信代码。生产 worker 运行在隔离容器或 microVM：无宿主凭据、默认断网、只读 bundle、限定素材目录，并限制 CPU、内存、PID、磁盘和时间。

默认不关闭 Chromium
sandbox，也不能因为容器启动困难就自动关闭它。只有独立审计过的外层 container 或 microVM 已承担等价 process
isolation 时，部署 adapter 才能显式选择关闭；该选择必须写入 locked
capture-environment
identity，并先在真实目标环境中通过 conformance。FFmpeg 参数使用数组而非 shell。远程素材下载处于独立 fetch 边界，限制 URL、重定向、大小和类型。

## 12. 交付关卡

### 第一关（已完成）：稳定渲出一条真视频

唯一目标是证明核心闭环：

```text
Screenplay → Timeline IR → Browser Runtime → Chromium → FFmpeg → MP4
```

范围只有：最小剧本语言、冻结素材 catalog、素材探测、Rust 时间求解、versioned
Timeline IR、不可变 presentation bundle、TS 确定性时钟、FrameReady
handshake，以及一个 whole-film Render
Unit 的真实视频。Gate 一验证了视频 seek、异步稳定、捕获方式和音频 mux；字体与图片的更多组合仍是 presentation 能力实验，不构成已冻结的 Gate 一语言表面。Gate 一执行并 mux 作者写下的 voice-over，不能静默丢弃音频。

native 一致性测试会比较独立 browser session 的 canonical raw-RGBA
fingerprint。退出一致性测试还会构建 release
CLI，将同一份剧本渲染两次，分别验证每份 H.264/AAC 输出的帧数、画面运动、stream
facts、音频落点，并证明最终发布不会覆盖已有输出。它不会把两次独立有损编码的 MP4 误当成 raw-frame
identity contract。

这一关不建设 coordinator、lease、远程 worker、能力调度和分层缓存。

### 第二关（已完成）：正确地切开并总装

已完成的切片把同一影片编译成两个独立的本地 Render
Unit，经既有 executor 捕获并总装。native 一致性测试会在编码前比较 whole-film 与 partitioned
canonical raw-RGBA sequence；release
CLI 一致性测试则分别验证总装后的 H.264/AAC 输出帧数、画面运动、stream
facts 与首个音频 packet 落点。它实现 Render Graph 与 `evaluation/output`
区间。该关最初延后了转场预卷与持久复用；后续 milestone 已经复用经过验证的
`FrameArtifact`，把失效范围收窄到已证明独立的 shot region，并接纳具有 widened
evaluation 的显式 transition overlap region。persist 与依赖历史的 region 仍然延后。

### 第三关（已完成）：离开本机仍然成立

已完成的数据面切片把本地使用的同一份 deterministic、versioned worker
request 投影到有界的 Lambda/S3
adapter。退出一致性测试先把一条含媒体的双镜头影片作为远端 whole-film
reference 捕获，再让两个 graph partition 并发进入独立 worker，比较 canonical
raw-RGBA frame sequence，并通过共享 H.264/AAC path 总装已校验的 artifact。S3
transport retry 与 conditional compare-and-verify
publication 是有界 adapter 语义，不是 distributed retry policy。canonical
Timeline IR 与 Partition Plan wire encoding 要等真实 external
consumer 出现后再实现；不预设 MP4 容器字节必然一致。

退出 harness 也是 Gate 三完整的 orchestration proof：一个短生命周期 owner 上传 immutable
input、调用有限数量的 worker、下载并验证 artifact，再完成总装。Gate 三不依赖数据库、queue、lease
service 或长驻 coordinator。后续 distributed-incremental exit proof 会直接复用一份已完整验证的
partition，不 materialize input 或准备 Chromium；随后故意破坏 disposable object，证明 conditional
repair 后会重新 capture。上述 proof 完成后部署工作冻结；provider workflow、公开 remote render
command、infrastructure definition、Lambda artifact publication 和其他 cloud
adapter 都必须等待新的真实需求，不属于 Gate 四或 Gate 五。

### 第四关（已完成）：作者音频与字幕

本关在不削弱精确时间与分片等价性的前提下，让通用音频和用户提供的字幕文件通过现有本地 compiler
与 renderer。任何 screenplay 新拼写都先用真实 eval asset 与 conformance fixture 满足语言准入规则。退出契约为：

- narrative voice-over 与通用音乐、音效保持不同语义；
- 外部 TTS 音频只是普通 frozen authored asset，不引入在线生成副作用；
- SRT、WebVTT 与 ASS 输入在进入浏览器前经过有界解析，并归一化成 Rust-owned caption fact；不支持的 ASS
  语义必须明确拒绝，不能静默丢失；
- audio placement、gain、duration、subtitle timing 与 caption text 都是 compiler 或 media 的精确事实，浏览器不拥有第二条时间线；
- malformed external file 产生带 source location 的 authored diagnostic；不可用或不可读文件仍是 typed infrastructure error；
- 一条本地含媒体影片同时覆盖跨 shot audio bed、shot-local sound、voice-over 与 imported caption，并证明 whole-film
  和双分片结果等价；关卡关闭时同时比较 canonical raw-RGBA frame 与 decoded audio timing/content。

固定 Linux real-process suite 已通过 native renderer 与 release CLI 验证这条完整链路：whole-film
与双 Render Unit 在携带 film music、shot-local effect、voice-over 和 imported caption 时，canonical raw-RGBA
frame 与 decoded audio 仍然等价。Gate 四没有加入 cloud conformance、deployment command、subtitle editor、speech generation 或 animation adapter。

### 第五关（已完成）：确定性的浏览器 effect

本关先做有界的 CSS、GSAP 与 Three.js 实验，再接纳 production API。退出契约为：

- 整数 `RuntimeFrame.index` 仍是唯一 frame identity；browser seconds 只用于设置 effect playhead，不能成为第二个时钟；
- paused WAAPI animation、paused GSAP timeline，以及 Three.js `AnimationMixer` 配合显式 render，在顺序与乱序请求 frame 时都复现相同像素；
- checked experiment 会在相互独立的锁定 Chromium process 中重复 non-monotonic sequence，并比较 canonical raw-RGBA fingerprint；
- 接纳后的 frame-effect boundary 必须在 `Seek(frame)` 内运行，并在 `FrameStaged(frame)` 前完成；不得创建第二个 scheduler、free-running clock、隐藏 queue 或无界 readiness wait；
- bundle metadata 携带一个由 `@onmark/runtime` 拥有的封闭 temporal capability。未知 presentation code 默认 sequential；只有 conformance 证明任意请求帧只依赖 immutable input 与该精确帧的 adapter，才可声明 random access；
- Render Graph 必须在分片前消费该 capability。凡能力允许切分，whole-film 与 multi-unit capture 的 canonical raw-RGBA sequence 必须相等；
- 官方 WAAPI、GSAP 与 Three.js integration 是 vendor-free runtime clock 之上的 vendor-specific code，不得成为 `onmark-core` 或 `@onmark/runtime` 的依赖。

Gate 五不增加 screenplay animation 拼写，不从 source inspection 猜 capability，不虚拟化 ambient wall-clock API，也不承诺任意 component 可 seek。这些能力必须在本关之后分别取得语言或 adapter 证据。

checked WAAPI、GSAP 与 Three.js playhead 全部通过标准
`PresentationRuntimeAdapter`：effect 在 `Load` 时绑定一次，在 `Seek(frame)` 内按声明顺序 apply，
并在 `FrameStaged(frame)` 前完成。dispose 是 terminal 的，单个 cleanup 失败后仍会尝试释放全部
owned effect。当前 bundle manifest 把封闭 capability 纳入 content identity。production
authored-HTML surface 已准入 random access：其 contract 禁止隐藏时钟，并要求每个 frame
effect 只根据 immutable input 与请求 frame 推导状态。未知的未来 browser component 仍默认
sequential，直至单独准入；低层 bundler 因为还要构造 conformance artifact，仍要求显式
capability。real-process conformance 会 bundle 一条跨 scene 的 GSAP presentation，分别作为
whole-film unit 与两个独立 unit 渲染，并比较完整 canonical raw-RGBA sequence。另一条
media、audio 与 caption fixture 会独立证明视觉与 decoded-audio 等价，再用共享路径组装最终
输出。

### 第六关（已完成）：确定性视觉资源与组件绑定

本关补齐 browser resource 地基，再允许后续性能关卡改变 capture path。本地 image、SVG 与 font
bytes 作为带稳定 identity、明确 resource fact、字节上限且不依赖 ambient network fetch 的 frozen bundle
resource 进入既有管线。browser runtime 为 video、image decode、font load、texture upload 与显式注册的
custom resource 提供一条 typed、bounded readiness boundary；超时必须指出仍未就绪的 resource 与 phase，
不能退化为匿名 presentation promise。static image admission 会在 resource 到达本地或 worker
Chromium 前，拒绝会自行推进的 raster container 与 SVG behavior。

native browser adapter 会通过 CDP request interception 执行这条约束，而不是依赖 presentation
自觉。Chromium 只允许读取 materialized private Unit Root 下 canonical file，以及内存中的 `data:`、
`blob:` URL；ambient network scheme 和逃出该 root 的 file path 都会在解析前被拒绝。Chromium
可能在 policy reply 到达前让一条 paused media request 失效；resource guard 只退役这条 stale
request，不会随之终止，其他 CDP failure 仍保持 terminal。本地与 worker 执行共用同一条策略。

Presentation binding 同时获得由 Rust 分配的 unit-local node identity、authored semantic
identity 与 parent relationship，以及通过 protocol 校验的封闭 properties、solved interval
与 frozen asset reference。Rust 继续独占 timing 与 resource fact；TypeScript 只决定这些
fact 如何成为 DOM、CSS、Canvas 或 WebGL。本关不引入自由 `start`/`end`、第二个 scheduler、任意网络访问，
也不通过扫描 source code 推断 temporal capability。image、component selection 或 properties 的任何新
screenplay 拼写，都必须先提交语言准入所需的 cases、prompts、grader、raw output 与保留 baseline。

退出 conformance 使用一条同时包含 font、image 或 SVG、video、caption、authored audio 与一个已接纳
frame effect 的本地影片。相互独立的 cold Chromium process 必须得到相同 canonical raw-RGBA sequence；
每种允许分片的 capability 也必须与 whole-film capture 等价。missing、changed、oversized、undecodable
或 unready resource 必须通过能指出具体 resource 的 structured bounded error 失败。checked bundle
必须保持 content-addressed 与 self-contained。

本关不加入 parallel browser capture、lossy screenshot transport、hardware encoding、layered native-media
composition、encoded worker segment、新 cloud deployment、transition、playback-rate control、component
marketplace 或 Studio。这些能力必须等待本资源契约完成后，再进入独立的 measured performance
或 language gate。

### 第七关（已完成）：经准入的分层原生媒体合成

本关只允许对显式声明封闭 visual-separability capability 的 presentation 改变权威像素路径。扫描源码、
video 列表为空或透明截图成功，都不能证明该 capability。未声明的 presentation 继续使用现有
Chromium-media 路径。

候选路径不改变 Rust 独占的 timing 与 placement fact。Chromium 只渲染透明 presentation layer；一条
persistent native media process 在 backpressure 下连续 decode、compose、fingerprint 并 encode 对应的
base frame。browser capture 与 native composition 必须形成一条有界 stream。生产实现不得落盘一整套按帧
编号的 PNG 目录，不得无界缓存 frame sequence，不得每帧启动 decoder，也不得在 native 与 Chromium
decode/color path 之间静默回退。本地与远程 worker 必须消费同一 Render Unit 与 executor path。

production admission 起步时只接受恰好一个主视频：它的 solved placement 必须等于 published
interval，冻结的 source dimensions 必须与 output profile 一致，完整 color tuple 必须是 BT.709
limited range。这是一条 layout proof，不是永久的全屏约定；更广的 `cover`、`contain`、crop 与
transform 必须先变成显式 typed fact 并取得独立证据。Rust 不猜这些 CSS 决策。声明的 capability
允许两种 execution plan：materialization 只在这些事实证明 native path 时记录
`SeparableOverlay`，否则记录 `BrowserComposite`。这是 launch 前的确定性 planning，不是 runtime
fallback；worker 必须逐字执行 transported choice。

native 帧率转换不得继承 `FFmpeg` 的默认 `fps` rounding。候选路径必须使用 Rust 独占的 source/output
有理帧率，把每个 source PTS 投影到第一个以帧中心选中该源帧的 output frame；`FFmpeg` 只能根据这些显式
PTS fact 丢弃或复制 decoded frame。锁定的 24→30 与非零 partition 检查负责防止这条 execution policy
长成第二套 timing solver。

实现意图不等于准入证据。候选路径进入 production capability 之前，一份 checked、locked Linux 实验必须
同时证明：

- 两个相互独立的 cold run 在候选路径内得到完全相同的 canonical frame fingerprint；
- whole-film 与所有被允许的 partitioning 在候选路径内得到完全相同的 canonical frame sequence；
- 一份具有完整 range、primaries、transfer 与 matrix 声明的受控色彩 fixture，在每个抽样 patch 上都满足
  固定为 4 个 8-bit level 的逐 channel 误差上限；缺失、不完整或不支持的 color fact 必须拒绝候选路径，
  不得猜测；
- admitted CFR profile 的 source-frame selection 仍然精确，包括非零 partition 起点与帧率转换产生的重复源帧；
- 在同一锁定机器上，以 1,920×1,080、30 fps、60 帧运行至少五次，端到端 wall time 中位数不超过现有
  Chromium-media baseline 的一半，process-tree incremental peak RSS 中位数不超过 baseline 的 85%；
- measured interval 必须包含 browser launch、readiness、全部 frame transport、native composition、canonical
  fingerprint 与 final encoding。不能因为某阶段两条路径共用，就把 startup 或该阶段排除在外。

实验必须记录 tool identity、machine profile、fixture identity、raw sample、median 与 rejection reason。共享
CI 负责 correctness 与 bound；容易受噪声影响的 performance admission 只在 pinned environment 运行，并将
reviewed evidence 提交进仓库。全部门槛通过后，才允许增加 versioned、explicit capability 及其 conformance
fixture。任何一项失败，实验继续保持 opt-in，production path 不变。
本路径的 capture-environment identity 除 Chromium、font、launch policy 与其他会改变像素的 host fact 外，
还必须覆盖 pinned `FFmpeg` binary 与 composition policy。

经过 review 的准入测量、production commits 与 closing CI 证据统一归
[`conformance/evidence/layered-media-admission.md`](../../conformance/evidence/layered-media-admission.md)
所有。production branch 在一个 local render sequence 内保留同一 compositor。分布式
frame-artifact capture 拥有一条容量为一的 output queue 与一个显式 `FFmpeg` framesync
lookahead；本地 MP4 encoding 依靠有界 stdin pipe 提供 backpressure，不会物化第二路 raw
frame output。历史样本与准入 revision 不再复制进长期架构合约。

Gate 七当时没有加入 VFR、新 codec、HDR、hardware acceleration、lossy screenshot transport、parallel browser
capture、transition、playback-rate control、Studio、component marketplace 或新的 screenplay
拼写；它们仍属于独立的 measured gate 或 language gate。

### 第八关（进行中）：闭合创作反馈，并扩展经过测量的媒体交付能力

本关先把现有 compiler 与 renderer 已经拥有的事实变成正式产品表面。`check`
在不启动 Chromium 的前提下验证作者源码、素材、presentation resource 与 render planning；
`inspect` 以稳定的人类文本和带版本的机器格式呈现 solved timeline、dependency region、
execution choice 与 cache identity；`doctor` 报告准入的 browser、媒体工具、capture mode
与平台策略。render progress 与 benchmark 使用同一组具名阶段和有界测量；这些命令都不能创建第二套
compiler 或 planner。

`doctor` 不会仅凭 executable bit 或零 exit status 推断 readiness。它并行运行四个十秒
有界的 handshake：browser、`FFmpeg` 与 ffprobe version probe，以及 bundler help
contract。每个 handshake 都校验 role-specific signature，并从每条 pipe 最多捕获
64 KiB，且不会把这些输出转发到 command output。每个 child 都有 kill-on-drop 与五秒
显式 cleanup bound，因此“可执行但角色错误”的文件不能被报告为 admitted toolchain。

交互式 `render` 会在 `prepare`、`bundle`、`plan`、`capture` 与 `assemble`
开始和完成时报告进度；redirected 与 JSON 输出不混入进度文本。`benchmark` 在私有
workspace 内执行一至九次有界奇数样本，强制使用 ephemeral frame artifact，使每份
样本都测量完整 capture，并报告所有阶段样本及中位数。它直接调用生产 render pipeline，
不得替换成缩水的 benchmark-only executor。

`snapshot` 在不引入 preview runtime 的前提下闭合第一条精确视觉反馈链。planning、region
bundling、visual admission 与跨 region 的 visual-path normalization 必须先与完整 render
一样完成；之后 CLI 才消费负责发布目标绝对帧的既有 region，并把 output 收窄为这一帧。
evaluation bound、selected shot、presentation byte、media dependency 与已经选择的
browser/native path 都不得变化。capture 仍写入普通 verified frame-artifact contract，
再读取其中唯一一张经过校验的 PNG 与 raw-RGBA fingerprint，并以 no-clobber 方式发布 PNG。
默认目标为 `renders/<screenplay-stem>-frame-<index>.png`；JSON 同时报告所属 region、
evaluation/output bound、shot index、capture mode、graphics backend、reuse、fingerprint
与 phase timing。

这是一项本地 authoring surface，不是 distributed task shape。它不会创建第二套 renderer、
final video encoder、remote single-frame request、contact-sheet scheduler、visual scorer、
approximate frame 或 black-frame fallback。它的 conformance claim 是：在同一个 locked
capture environment 中，与完整 production artifact 的对应帧具有相同 canonical
raw-RGBA。

`review` 闭合更宽的精确反馈循环，但不会演变成 Player 或 preview server。一个纯 CLI
策略选择每个 Render Graph region 的首帧、中间帧与最后一张已发布帧，再加入由已求解
shot、transition、video、overlay 与 imported caption 产生的视觉边界；相同 frame identity
会合并成一个 checkpoint。策略是确定性的；超过 512 个 checkpoint 时直接拒绝，而不是静默
抽样；也不会读取 presentation source 来猜测视觉重要性。

review capture 按原计划执行所有普通 production region，不把它们收窄成 review-only
Render Unit。已经验证的桌面 `FrameArtifact` 会直接复用，只有 cache miss region 才进入
Chromium。每个选中 frame 只从其所属 verified artifact 读取一次，并发布为 lossless PNG。
一份静态 HTML contact sheet 和 versioned JSON manifest 会记录精确 frame、所属 region、
evaluation/output bound、shot dependency、semantic checkpoint reason、source span、timing
provenance、frame-artifact identity、PNG digest 与 canonical raw-RGBA digest。report 不含
wall-clock measurement 或 mutable cache state。

默认 review directory 由这份 canonical manifest byte 进行内容寻址。只有 manifest 与每个
已命名 PNG 都通过有界完整性检查，既有 report 才能复用。可选的 prior manifest 只作为比较
输入：它报告未变化、变化、新增和删除的 region artifact，但不会进入当前 report identity。
artifact reuse 始终由已有 production cache key 决定，因此 comparison 不能授权复用；语义上
已变化的 region 也不能仅凭某个 checkpoint 看起来相同而继承旧 pixels。

这个 surface 刻意保持静态和本地。它不会增加 playback clock、scrubber、hot-reload server、
source mutation、visual scorer、sparse distributed frame task、第二套 timing solver、
approximate frame 或 hidden capture fallback。它的精确性来自 production artifact contract
本身，而不是与另一套 preview implementation 做相似度比较。

第二个切片只通过 typed fact 与锁定证据接纳更广的媒体输入、输出 profile 与 native placement。
VFR 输入保留冻结的源字节，并要求完整的 frame timestamp map；Onmark 不会把它转码成隐式 CFR
替身，也不会让 browser 或 `FFmpeg` 默认值选择 source frame。额外 codec 必须在同一份冻结字节上
获得 decoder 与 color contract 的准入证据。透明或其他 container 输出必须端到端保留请求的
pixel contract。native crop、scale、picture-in-picture 与 multi-video placement 必须先具有
显式 layout fact，并通过 whole-film、partitioned 与 distributed raw-pixel equivalence，才能
绕过 Chromium。

获准的 browser-backdrop path 显式声明为 `separableBackdrop`；authored presentation
没有这项声明时仍是 `browserComposite`。planning 只接纳 1 到 16 个 CFR、BT.709
limited-range video placement，并要求其 source treatment 已被 native path 支持。声明的
candidate 若无法通过 admission 会直接失败，不会静默送回 Chromium。

browser 独占 CSS evaluation，但不会成为第二个 planner。capture 之前 executor 以
`layoutOnly` mode 加载 unit，并接收一组经过 protocol versioning、按 node 排序的整数
viewport rectangle，以及封闭的 `object-fit` 和 fixed-point `object-position` fact。
Rust 用 Render Unit 里的预期 media identity、source dimension、output profile 与 solved
interval 校验这份 evidence。只有精确且不越界的 crop/scale 算术可以通过；native rectangle
若同时在空间和 solved interval 上重叠则被拒绝。校验后的 placement 会成为该 capture
transaction 唯一且不可变的 `BackdropLayoutPlan`。
shot projection 会在零 native media 的 browser-only region 上保留该 capability。这类
region 单独执行时正常走 browser capture，与同一 sequence 中包含 native media 的 region
仍保持兼容；本地 media set 为空不会改写其 visual contract。

随后一条连续且有界的 `FFmpeg` process 把 Chromium 的 opaque 或 alpha-preserving output
作为 canvas，再以显式 trim、PTS offset、crop、scale 与 destination coordinate 叠加每个
time-bounded native source。它既不拥有 screenplay timing，也不解释 CSS。local sequence、
worker artifact 与 distributed assembly 共用这条 executor branch；不存在另一套 worker
compositor，也不会逐帧启动 decoder。

这条 branch 冻结的是 CSS geometry，不是 Chromium 的 media-rasterization algorithm。
通过 admission 后，video pixel 由 locked native decoder、color conversion 与 scaler
定义。conformance 因此要求这条 branch 的 whole、partitioned、local 与 worker
execution 拥有相同 canonical raw-RGBA；它不宣称与另一种 `browserComposite` pixel
ownership contract 逐字节相同。

cache identity 刻意记录能确定 browser layout 的输入，而不重复保存它的派生 response：
bundle identity、Browser Plan、render profile、预期 native-media plan 与 capture
environment。若把 response 本身加入 key，每次 cache lookup 前都必须先启动 Chromium，
会直接破坏 warm incremental reuse。capture environment 会锁定 browser、native compositor
policy 与 binary；cache miss 在发布任何输出前冻结并校验派生 evidence。

仓库中的
[`backdrop-layout-admission`](../../conformance/evidence/backdrop-layout-admission.md)
记录包含 whole/partition/worker 的 raw-pixel 证明，以及首轮同机
browser-versus-native 性能样本。目前 measured native candidate 与 browser composition
接近，但并没有更快；后续任何性能优势声明都必须重新进行锁定实验。

获准的替代输出是面向剪辑且保留 alpha 的 MOV：`ProRes` 4444
（编码为 `yuva444p10le`，锁定工具链解码为 `yuva444p12le`）配 48 kHz
双声道 24-bit PCM。原有交付配置仍是 MP4 中的不透明 x264 H.264
（`yuv420p`）与 AAC。一个封闭的 `EncodeProfile` 在浏览器直出、原生分层、
本地组装和分布式帧产物组装之间统一拥有视觉编码器、音频编码器、像素格式、
容器、暂存后缀与机器拼写。CLI 只从 `.mp4` 或 `.mov` 扩展名选择一次，并拒绝
其他拼写；不得让 `FFmpeg` 默认值暗中决定结果。桌面发布验收会从已安装
产品分别渲染并探测两种配置。

alpha 保留是影响像素的 `RenderProfile` 事实。它在导航前选择透明 Chromium
根表面，进入 worker request 与 frame artifact identity，并在最终编码时不
偷偷铺黑底。准入测试比较整片捕获与独立分片 artifact，经生产 encoder
重新组装，要求 raw RGBA 序列相等、两个 MOV 都探测为 ProRes 4444，并同时
保留全透明和半透明像素。不透明 MP4 保持独立的浏览器表面，避免 H.264
隐式压平透明抗锯齿。

media treatment、transition、dynamic author input、caption presentation 与 multiple subtitle
track 一旦改变作者语义，就属于语言工作。每项新增能力都必须先提交语言准入规则要求的 cases、
prompts、grader、raw model outputs 与 baseline。之后 trim、rate、gain、fade、dependency 或
transition interval 由 Rust 独占；TypeScript 只能实现已经求解的视觉 effect。JavaScript
timeline、CLI flag 或 `FFmpeg` filter string 都不能成为另一套 scheduler。

Gate 八在比较 declarative HTML binding、module-owned binding 与 source placeholder
三条生成路径后，接纳 canonical typed variant。三条 arm 都完成了锁定的十二个 case；
declarative binding 使用 5,708 authored bytes，module binding 使用 8,723，placeholder
使用 5,443。获准方案以 265 bytes 的极小代价保留 readable default、static dependency、
parse-once value、可复用 bundle 与 literal sink，同时不引入 executable input code 或
source rewrite。该结论由 checked-in `evals/typed-variants` 资产拥有。

实现保持一条由 Rust 独占的 linear path：

1. compiler bind 识别一份 film-local `om-fields` declaration 与带 source 的
   `data-om-*` binding；
2. resolve 只解析一次封闭的 text、integer、boolean 与 color domain；
3. 有界 flat-JSON reader 校验一份可选 override document，并产出 immutable canonical
   value；
4. Timeline IR 记录 schema 及每个 field binding 的精确 semantic scope；
5. Render Graph 为每个 dependency region 选择真实需要的 field；
6. Browser Plan 只携带该 region 的一份 name-sorted value vector；
7. runtime 在 motion prepare 之前只应用 `textContent`、CSS custom property 与
   `hidden`。

该能力不新增 crate、package 或 production parser dependency。variant value 是
`onmark-core::model` 的 foundational domain value；source declaration、external JSON
diagnostic 与 binding resolution 留在 compiler module。小型有界 JSON reader 用于保留
generic deserialize-to-map 会丢失的 duplicate key 与精确 source span。protocol 仍独占
wire projection。TypeScript 不校验 author JSON、不从 DOM 反推 field scope，也不创建第二套
value model。

field dependency 与 document projection 对齐，但始终是 typed compiler fact。film-shell
scope 选择全部 region；scene-shell scope 选择保留该 scene 任一 shot 的 region；shot scope
选择保留该 shot 的 region；transition scope 只选择同时保留两个相邻 shot 的 region。同一
field 的多个 scope 按 union 合并。精确 matcher 属于 Render Graph planning；会让
transition variant 过度失效的近似“field touches any shot”集合不予准入。

immutable browser bundle 只包含 fallback markup 与 binding declaration，不包含 variant
value。Browser Plan identity 已进入 Render Unit 与 `FrameArtifactId`，因此 field 改变只会
使真实携带它的 region 失效。本地 execution、worker execution、distributed reuse、review、
snapshot 与 final assembly 共用同一 plan，不存在 provider-specific variant path。已声明但
未使用的 field 是 warning，也不会进入 artifact identity。

所有会编译 presentation output 的 authoring surface——`check`、`inspect`、`snapshot`、
`review` 与 `render`——都接受一份有界 external variant document。一份 versioned batch
manifest 命名 screenplay、与 profile 无关的 variant document 及 output；CLI 只 resolve
一次 screenplay、只 freeze 一次 asset，串行执行有界数量的 variant，并通过现有 cache 复用
未变化 region artifact。串行执行使 browser 与 encoder 的资源上界等于一次普通 render。
manifest 属于 orchestration input，不是 screenplay syntax，也不能覆盖 timing、asset、
capability、render profile 或 output dimension。

一次 1,920 × 1,080 production-campaign 实验在同一份 435-frame screenplay、共享
subtitle track、frozen asset set 与 7-region partition plan 上渲染 20 个 variant。精确
dependency scope 复用了 8,700 个 frame instance 中的 5,790 个，以及 140 个 region
instance 中的 84 个。film-shell edit 会使整个 plan 失效；shot 与 transition edit 会保留
无关 artifact；已经缓存的 composed variant 会复用全部 7 个 region。该 workload 暴露并
补上了 shared batch-subtitle import、boolean visibility ownership、fractional-frame
GSAP boundary 与 transition-trimmed video coverage 的回归保护。

该实验刻意区分 cache identity 与 independent cold pixel identity。首轮实验暴露了
Chromium tiled GPU raster path 的 cold-process 漂移；exact raster follow-up 关闭 GPU
与 partial raster、锁定 baseline Skia code path 后，两个独立 cold session 的 435 帧
canonical raw-RGBA 全部一致。cross-batch persistent reuse 只对这份锁定的 software
contract 准入。完整结果记录在 `conformance/evidence/variant-campaign.md`。

该能力不增加 template engine、string substitution、global/URL input object、source
mutation API、remote authoring、coordinator 或 mutable runtime update channel。一份 Render
Unit 内的 value immutable。会改变 temporal capability 与 cache identity 的逐帧动态 input
和 branch 仍然延后。

Gate 八已准入的视频处理能力始终只描述素材局部。第一组 live-model 实验在全部二十个
编辑 case 中准入 `trim="起点..终点"` 与精确 `speed`，没有引入 film 坐标；range
拼写也比两个边界属性使用更少 authored bytes。第二组 20/20 实验准入语义无歧义的总播放
次数 `plays` 与最终帧停留 `hold-last`，并拒绝给 HTML 的布尔 `loop` 拼写再赋整数含义。

resolve 对每项 treatment 只解析一次，solve 使用整数有理运算推导输出帧，Timeline IR
独占完整 source mapping，Browser Plan 把该事实传给确定性的 CFR 或 VFR 选帧。原生合成
在 Chromium 对照证明 source-frame selection，且彼此独立的本地与 worker 分区产出相同
native raw-RGBA sequence 后准入 trim 与 speed；它不要求已知不同的 Chromium 与 `FFmpeg`
decode/color path 得出相同 hash。重复播放与最终帧停留在取得同样独立的 native 证据前仍走
browser composition。runtime 只在接纳不可信 plan 时重复校验 wire-level duration
invariant，不推导或修改 authored timing。

Gate 八还在 checked-in 生成对比保持 20/20 可靠性后，接纳了显式
`<om-transition duration="…"></om-transition>` boundary。bind 要求 marker 位于同一
scene 的两个相邻 shot 之间；resolve 只解析一次正的精确 duration；solve 独占 overlap，
并拒绝无法容纳的 window。Timeline IR 记录这条事实，Render Graph 将其拆成互不重叠的
output region，同时把 overlap region 的 evaluation 扩大到两个 shot。partition 的精确
shot set 通过 `BundleProjection` 交给 bundler，并通过 Browser Plan 交给 runtime，因此
本地、增量与分布式执行消费同一关系。TypeScript 只接收已求解 interval 与相邻 DOM
element 来实现像素，不选择 window，也不推导 graph dependency。

Gate 八不加入 Player、Studio、preview server、source-mutation API、component marketplace、
remote authoring command、coordinator、database、queue、lease service、cloud workflow、
infrastructure definition 或新的 provider adapter。Agent integration 只是稳定 CLI diagnostic
与 inspection 之上的薄 skill；它可以教授工作流，但不能隐藏重试、静默自更新，或用 prompt 文本替代
compiler policy。
仓库中的 `skills/onmark-video` 通过开放 Agent Skills 目录布局分发。它不包含 executable
helper、模板、复制的语言规范或私有 render path；安装它的 agent 必须通过已发布 CLI 闭合
反馈循环，并把 versioned JSON diagnostics 与 inspection 视为权威事实。

每一关都使用最终方向的 IR 和协议，但只实现本关真实消费的部分。上一关没有稳定通过，不创建下一关的空架子。

## 13. 待实验决策

Gate 七之后仍需分别完成 Windows native-graphics admission、desktop default
policy 与 capture-environment identity 粒度的测量；macOS 的 opt-in
raw-RGBA conformance 不能替代这些跨平台证据。

Gate 一首轮 capture spike 得到了正向但刻意收窄的证据：页面自行控制
`FrameReady`，随后调用 CDP
`Page.captureScreenshot`，DOM/CSS/Canvas 帧在同一锁定机器的独立 Chrome 进程间得到一致的 raw
RGBA hash。Gate 三在标准 Linux path 上把这条临时 transport 替换为
`chrome-headless-shell` BeginFrameControl，使 compositor commit 与 screenshot
共享一个显式帧边界。portable screenshot backend 已通过独立 whole-film session、
decoded output 检查与 canonical raw-RGBA 比较，在锁定的 macOS 与 Windows release
target 上获得准入。这不承诺不同操作系统、browser product 或 capture mode 之间像素相等。

decoded-media 实验现已覆盖 30 fps CFR、`30000/1001` CFR 与交替帧间隔 VFR
H.264；三者都使用 30 帧 GOP、3 个 B-frame，并按 `17 → 3 → 29 → 17`
乱序 seek。`requestVideoFrameCallback` 在 capture 前注册，并在 `BeginFrame`
后通过 `mediaTime` 确认 captured source
frame，随后才返回 FrameReady；VFR 期望来自 ffprobe 的真实 source-frame
timestamp，不假设 source/output frame 对齐。两个独立 Chromium session 的 PNG
capture byte-identical，同一 source-frame timestamp 的独立 FFmpeg
extraction 也在重复执行间 byte-stable。实验同时发现：把精确 CFR 帧边界秒数直接写入
`video.currentTime` 会选中前一帧，必须采样 Rust 已选帧内部。

两条 decode path 并非 pixel-interchangeable。四张 320×180
RGBA 帧共 921,600 个 channel，Chromium canvas 与 FFmpeg raw
extraction 约有 229k–232k 个 channel 不同，mean absolute
delta 为 2.13–2.18，孤立最大值为 173–178。当前机器上 browser
seek/readiness/screenshot 平均 51–81ms/帧；每帧单独启动 FFmpeg 的 native
extraction 为 18–19ms，但后者尚未包含 browser
injection、composition 与最终 capture，因此不能当成端到端速度胜负。Gate 一的一次 render 必须只认一条 decode/color
path，并把它纳入锁定环境；多 codec/色彩、更长随机序列、persistent native
decoder 成本与 injection overhead 仍需测量。

后续 Linux arm64 A/B 测了完整 pre-extraction
alternative，而不再只测 process-per-frame extraction。同一份生成的 30 fps H.264
source，在锁定的 v149 headless shell 中顺序渲染 60 张 1,920×1,080 帧。native
browser seek 加 `BeginFrame` capture 用时 3.89 秒，process-tree incremental RSS
peak 为 292 MiB。单次 `FFmpeg` 7.0.2 extraction 用 0.23 秒生成 23.4 MB lossless
PNG；同一个复用 browser image 依次加载这些文件并 capture
60 帧还需 2.34–2.38 秒，但 repeated sample 的 incremental RSS 达到 944–949
MiB。抽样四帧的 33,177,600 个 RGBA channel 中有 16,665,272 个不同，mean absolute
delta 为 7.21，最大值 198。因此当前明确拒绝把 pre-extracted PNG
injection 作为默认路径：约三分之一的 latency 收益不足以抵偿三倍内存与隐式 decode/color
path 变化。未来只有 streaming native decoder 同时证明 browser
transport 有界、color policy 显式且端到端证据不差于现状，才重开该选择。

后续 Linux
arm64 实验继续验证了 streaming 形态，但不再把 media 注入 Chromium。同一份 60 帧、1,920×1,080
workload 先由 Chromium capture 稀疏透明 presentation
layer 并退出，再由一条 single-threaded `FFmpeg` 7.0.2 process 连续 decode H.264
base、合成 PNG layer 并流式输出 RGBA。透明 capture 为 1.16–1.22 秒，native
composition 为 0.27–0.34 秒，串行总计 1.46–1.52 秒；权威 browser-media
path 为 3.77–3.84 秒。两阶段的 incremental RSS peak 分别为 220–221
MiB 与 215–238 MiB，60 张透明 PNG 合计 2.96
MB。在两边使用同一份 Chromium-decoded base 时，straight-alpha
composition 的 33,177,600 个抽样 channel 中只有 6,240 个不同，mean absolute
delta 为 0.0002，最大值 2。显式把 source 标为 BT.709 limited
range 后，完整 native path 的 mean
delta 从 6.82 降为 0.67，但仍有 4,938,423 个抽样 channel 不同，孤立最大值达到 202，因为 Chromium 与
`FFmpeg` 并不共享同一套 decode/chroma reconstruction 实现。当时 layered
path 只证明了性能与内存候选价值，并未证明 raw-pixel equivalence。Gate 七后来只在
frozen asset metadata 拥有完整 BT.709 limited color tuple、且 bundle 显式声明
`separableOverlay` 后，准入了更窄的生产合约。planner 会在 launch 前用事实选择该
native path；否则选择 `browserComposite`。executor 绝不在运行中切换成隐藏 fallback。

当前视觉 profile 接纳具备一个精确 CFR rate 或完整 VFR timestamp map 的 H.264
素材。`browserComposite` 使用锁定 Chromium decoder 作为权威 decode/color path，且只有
`requestVideoFrameCallback.mediaTime` 指向 Rust 选中的 source frame 时才返回
ready。`separableOverlay` 只有在 Gate 七的 color、layout 与 source-treatment proof
成立且素材为 CFR 时，才使用已准入的持久 native decoder 与 compositor。不支持的 codec
与不完整 native-path fact 不会被静默近似：它们会被拒绝，或留在已经证明的 browser path。

这条策略由 render-owned `AdmittedVideo` proof 对 core-owned metadata 执行 admission
来表达。它借用规范化事实，不复制第二套 render 媒体模型，并证明 H.264 与完整 source
timing。Render Unit 保留该 timing，并只向每个 browser placement lower 一次；native
admission 再独立要求其中的 constant-rate 子集。decoded-media conformance 通过生产用
的有界 ffprobe boundary 同时取得 CFR 与 VFR 证据，whole-film executor 则通过 production
adapter 消费同一份被接纳的视频。

- 当前 native capture 已选择 headless shell 的 CDP
  BeginFrameControl；只有更强的正确性与性能证据才能重开 WebDriver BiDi、surface
  copy、编码流或其他 transport 选择；
- 分层 alpha 缓存何时值得额外成本；
- Timeline IR 与 Partition Plan 公开编码使用 JSON、Protobuf 或分层组合；
- subtitle style 如何归一化，同时不把不支持的 ASS 语义静默降级；
- 哪些动画适配器可随机 seek，哪些必须 warm-up/sequential；
- 浏览器、字体与 FFmpeg 环境锁定到什么粒度。

实验优先级依次为：捕获方式与 FrameReady 正确性、未知组件的保守执行成本、分片与预卷、跨 worker 一致性、分层缓存收益。纯编译内核、确定性协议、依赖驱动分片和本地/分布式同构是基础骨架，不应反复摇摆。
