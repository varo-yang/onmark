# Onmark Presentation Contract

> 状态：已覆盖 Gate 一至 Gate 六。

Gate 一使用两个作者文件：

- `film.onmark` 拥有剧本事实：结构、内容、ID、cue、素材引用和时间关系；
- `presentation.ts` 拥有浏览器 effect：DOM、CSS、layout，以及安装 runtime host。

这个拆分是有意的。剧本保持可朗读、可编译、由 Rust 拥有语义；presentation 获得正常 TypeScript 工程能力，但不能变成第二套求时语言。Rust 仍然拥有所有 interval。TypeScript 可以渲染已求解事实，但不能解析 cue、推导 shot 时长、规划分片或选择帧区间。

## 最小入口

Gate 一 presentation 通常长这样：

```ts
import { createDomPresentationBindings } from "@onmark/authoring";
import {
  PresentationRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
} from "@onmark/runtime";

import "./presentation.css";

const adapter = new PresentationRuntimeAdapter(
  createDomPresentationBindings({
    document,
    videoSource: materializedVideoSource,
  }),
  5_000,
);

installRuntimeHost(adapter);
```

`onmark render film.onmark` 默认寻找剧本旁边的 `presentation.ts`；也可以用
`--presentation` 指定入口。bundler 会编译该入口、注入固定版本的 Onmark
packages、生成一个不可变 browser artifact，并用 Rust-owned bundle
manifest 记录它。

## 公开 adapter 生命周期

runtime 只有一条浏览器 effect 边界。presentation 通过
`installRuntimeHost(adapter)` 安装实现：

```ts
interface RuntimeAdapter {
  load(plan: RuntimePlan): Promise<void>;
  prepare(frame: RuntimeFrame): Promise<void>;
  seek(frame: RuntimeFrame): Promise<void>;
  confirm(frame: RuntimeFrame): Promise<void>;
  dispose(): Promise<void>;
}
```

`load` 收到已接受 `BrowserPlan`
的递归冻结快照。它可以创建资源，但不能保留一份可变的author-owned plan。`prepare`
恰好在 `plan.evaluation.start`
运行一次，且只能在该帧所需资源稳定后 resolve。`seek`
只会在 prepare 成功后运行；它应用请求的 DOM 状态、预先注册 decoded-media
observer，并在媒体完成 seek 后 resolve，但不能等待 compositor presentation。
`confirm` 在 native capture 后运行，只有 browser media 证明 staged source
frame 已在 native 接受 captured
payload 前进入 compositor 才能 resolve。即使 cleanup 报错，`dispose`
也是终止相位。

`seek` 不接受自由时间 `t`，而是接收 `RuntimeFrame`：

```ts
interface RuntimeFrame {
  readonly index: number;
  readonly timeSeconds: number;
}
```

`index` 是 native executor 选择的绝对、精确帧身份。`timeSeconds`
只是经 Rust-owned 有理帧率推导出来、供浏览器 API 使用的投影；它不能成为另一套调度时钟或时间决策来源。

## Runtime 握手

presentation 必须用 `installRuntimeHost` 安装一个 runtime host。`Load`
会创建 plan 中的每个 video 与 overlay node。导入字幕是 caption role 的
overlay，与其他 overlay 共用已求解 visibility path，不另造 browser timing
engine。inactive node 保留稳定的 binding
identity，但在其 solved
interval 使其可见之前不进入 layout 与 compositor；这样 Render
Unit 之外的 placement 不会改变当前像素。 `Prepare` 之后，native
renderer 会在固定的 pre-baseline timestamp 发送并等待一次 visual、non-capture
BeginFrame，以初始化 page surface。真实 capture 使用更晚的固定正 compositor
baseline：

```text
Load(plan) -> Prepare(evaluationStart)
  -> native surface initialization without capture
  -> (Seek(frame) -> FrameStaged(frame)
      -> [native placement-boundary commit]
      -> native BeginFrame capture
      -> Confirm(frame) -> FrameReady(frame))*
  -> Dispose
```

这个拆分来自 Chromium decoded-media 的真实约束：`requestVideoFrameCallback`
必须在它要观察的 compositor frame 之前注册；但在 CDP BeginFrameControl
target 上，如果先等 callback 再发送 `BeginFrame`，两边会形成死锁。因此
`FrameStaged(frame)` 只表示 browser
state 已能进入 compositor。native 随后为每个 output
frame 发送一次正常的、同时 commit frame state 与 capture PNG 的
`HeadlessExperimental.beginFrame`。在 video 或 overlay boundary，native 会先在当前
compositor transaction capture tick 之前的固定亚毫秒 offset 发送一次无 screenshot
commit，让新可见 layer 获得一次 compositor turn，同时不保留无关 inactive
layer，也不推进剧本时间。compositor tick 严格按 capture 顺序向前；
`RuntimeFrame.index` 仍是 authored time，可以后退或重复。no-damage
response 通常复用上一张 PNG；boundary 绝不复用上一 placement，该情况与空的首帧 capture 都会获得一次有界的亚毫秒重试。`Confirm(frame)`
等待预先注册的 callback；在 placement boundary，observer 可能在 pre-capture
commit 上完成，而 runtime media
state 在该 commit 与精确 capture 之间不能改变。因此 `FrameReady(frame)`
表示精确 capture 的 staged media 已在 native 接受它之前通过 decoded-media
confirmation。placement boundary 随后会在该 transaction 的下一个正亚毫秒 tick
执行一次有界的 reconciliation capture；若 confirmation 没有产生新的 compositor
damage，Chromium 可以省略 pixels，native
便复用精确 capture，否则以新 pixels 替换。确认失败时，captured payload 在进入 encoder 或
frame artifact 前就会被丢弃。

## 所有权

边界必须清楚：

| Owner         | Owns                                                                |
| ------------- | ------------------------------------------------------------------- |
| Screenplay 与导入字幕 | authored 结构、文本、素材引用、cue、局部 delay                |
| Rust compiler | parse、normalize、reference resolution、精确求时、Timeline IR       |
| Runtime       | protocol 状态、frame clock、视频解码 readiness、visibility interval |
| Presentation  | DOM 形状、CSS、layout、字体与视觉风格                               |
| Renderer      | materialized asset path、Chromium、capture、encoding                |

presentation 收到的 placement 已经包含绝对帧区间。它可以决定 title 长什么样、CTA 放在哪里、video 怎么被 CSS 布局；它不能把 title 提前、延长 overlay、重新解释
`delay`，也不能从 DOM 里重新推导媒体时长。

## Authoring facade

`@onmark/authoring` 提供默认语义 DOM bindings：

- `createDomPresentationBindings({ document, videoSource, resources? })`
  返回 runtime 可消费的 video、overlay 与显式注册 presentation resource bindings；
- video placement 会变成隐藏的 `<video>`，并带稳定 class `onmark-video`；
- title、CTA 与 caption placement 会变成隐藏的 `<div>`，并带
  `onmark-overlay` 以及 `onmark-title`、`onmark-call-to-action` 或
  `onmark-caption`；
- runtime 根据已求解 interval 切换 visibility，CSS 拥有 layout。

默认 facade 刻意很小。presentation 可以直接实现 `PresentationBindings`
来支持 Canvas、WebGL 或自定义 DOM，但规则不变：binding 创建浏览器资源，`setVisible`
应用可见性，`dispose` 终止性释放资源。

更精确地说，production adapter 会在 `load` 时各调用一次
`bindVideo(placement, index)`、`bindOverlay(placement)`、`bindResources(plan)` 与
`bindFrameEffects(plan)`。video binding 提供浏览器 element、已 materialize 的 source、visibility effect
和终止性 cleanup；overlay binding 提供 visibility 与终止性 cleanup。video 的 `index`
只是它在冻结 unit plan 中的位置，不是时间坐标。每个 overlay 则携带 compiler-owned
`componentId`；即使更早的 overlay 没进入某个 partition，它也保持不变。每次 `seek` 时，runtime 先隐藏
video，再根据权威 output frame 选择已准入的 source frame、呈现 ready video，最后应用已求解 overlay 的 visibility。
binding 拥有效果，不拥有 interval arithmetic。

## Plan facts、组件选择与 props

当前语言**没有**
`presents`、`definePresentation`，也没有 screenplay 到 presentation 的 props 通道。`onmark render`
通过 `--presentation` 或同目录发现选择一个
`presentation.ts`。该 entry 唯一收到的动态事实，是 `Load(plan)`
传入的 Rust-owned `BrowserPlan`：帧率、evaluation/output interval、video
placement，以及 title、CTA 或导入 caption 的 overlay placement。
`presentation.ts` 静态 import 的值是 bundled program code，不是 screenplay
props。

这些既有 overlay fact 构成封闭的内建 component contract：`componentId`
是稳定身份，`kind` 只选择 title、CTA 或 caption，`text` 是该 component
唯一的 authored property。这不会创建通用 props 通道，也不允许 presentation
重新解释 screenplay 结构。

这项缺失是有意边界，不是未写下来的约定。未来的 presentation selection 或 props
feature 必须一起定义：screenplay spelling、带类型的 schema/default、canonical
wire encoding、source span 与 diagnostic、bundle/cache identity，以及与 temporal
capability declaration 的关系；它还必须具备受控 language
evaluation 证据。在这些工作完成前，presentation 不得从 global、URL
parameter、可变 side channel 或自造的 `presents` attribute 读取作者意图。

## Temporal capability

公开的封闭能力是 `@onmark/runtime` 拥有的
`PresentationTemporalCapability`。当前只接纳 `sequential` 与
`randomAccess`；`warmup(n)` 及更宽的依赖分类仍只是架构设想。CLI 会把未知代码默认成
`sequential`，底层 bundler 则要求显式传值。sequential 执行只生成一个 whole-film region。

公开的 `FrameEffect` boundary 由 `@onmark/runtime` 拥有。authoring 可以向
`createDomPresentationBindings` 提供 `frameEffects(plan)` factory；标准 adapter
会在 `Load(plan)` 时调用一次，并独占返回的 effect 直到 terminal
disposal。每次 `Seek(frame)` 中，effect 会在 solved video 与 overlay placement
之后按声明顺序 apply，所有返回 promise 都必须在 `FrameStaged(frame)`
前完成。effect 只获得精确 immutable `RuntimeFrame`，不会得到 scheduler 或 mutable
timeline。单个 cleanup 失败后仍会尝试 dispose 全部 effect。

这条 lifecycle 本身不是 random-access 声明。只有 conformance 证明任意请求帧只依赖
immutable input 与该精确帧后，presentation 才能以 `randomAccess` 构建。声明是显式 build
metadata，不从 source 或 screenplay spelling 猜测。当前 bundle manifest 把它纳入
canonical identity，Rust 在 Render Graph 分片前消费它；未指定的 CLI 声明始终按
`sequential` 处理，底层 bundler 则要求显式传值。

## Visual capability

`PresentationVisualCapability` 声明 Chromium 拥有哪些像素。它是 build metadata，
不是 screenplay spelling，也绝不从 presentation source 猜测。CLI 默认使用
`browserComposite`，底层 bundler 要求显式传值。

- `browserComposite` 表示 Chromium 拥有包括主视频在内的完整画面，是未知
  presentation code 的保守能力；
- `separableOverlay` 表示 Chromium 只产出与主视频像素无关的透明前景，native
  execution 可以先解码并安放主视频，再以 source-over 合成该前景。

声明 `separableOverlay` 的 presentation 在 browser video placement 被移除后仍必须
保持正确。它可以使用 solved interval、overlay fact、精确 frame identity 与 immutable
visual resource；不得把 video 采样进 Canvas/WebGL，不得读取 media pixel，不得使用依赖
背景的 filter 或 blend mode，也不得让前景像素以其他方式依赖下面的主画面。能力由
conformance 接纳，不能因为 source scan 暂时没找到禁用 token 就获得信任。

当前 native path 刻意比 presentation promise 更窄：必须恰好有一个覆盖完整 published
interval 的主视频，冻结的 source dimensions 必须与 output profile 完全一致，并且完整
color tuple 必须属于已准入的 BT.709 limited-range profile。这些检查避免 Rust 重造 CSS
layout。声明能力却不满足 native profile 时，执行会在启动进程前失败，绝不偷偷回退到
browser composition。

当前 Bundle Manifest 把 temporal 与 visual capability 都纳入 canonical identity。
bundle 是可重建产物而非 authored data；reader 只接受当前版本，旧 bundle 直接重建。

## 素材

浏览器只看 unit root 下已 materialize 的素材。Gate 一 video source 使用：

```ts
materializedVideoSource(placement);
```

这个 helper 从 Rust-owned browser plan 里的 frozen asset identity 推导
`./assets/sha256/<digest>`。presentation 不应该拼 native
path、读取源码文件或假设 working
directory。renderer 会在浏览器看到素材前验证字节。

Gate 六现已允许 presentation JavaScript 或 CSS import 本地 AVIF、GIF、JPEG、PNG、SVG、WebP、OTF、
TTF、WOFF 与 WOFF2 文件。bundler 会把原始 bytes 写入不透明的 `resources/` 路径，并纳入既有的有界、
content-addressed manifest。bundle 只证明 byte identity；browser readiness 还必须显式注册：

```ts
interface PresentationResource {
  readonly kind: "image" | "font" | "texture" | "custom";
  readonly id: string;
  prepare(): void | Promise<void>;
  dispose(): void | Promise<void>;
}
```

`resources(plan)` 最多返回 256 个 resource；其 `kind:id` identity 必须唯一、非空、去除首尾空白且长度
有界。`Prepare` 会在 adapter 的共享 readiness deadline 下并发启动全部 resource、等待全部有界结果，
并把所有超时 identity 报成 `<kind>:<id>:prepare`。未类型化的 preparation failure 也会被收敛到同一
identity。terminal disposal 按声明顺序等待所有 resource；即使一个 cleanup 失败，也不会跳过后续
resource，并只保留第一个 failure。
factory 在返回前仍拥有自己创建的 effect；如果构造到一半抛错，它必须自行释放这些 partial
effect。runtime 只接管已经返回的 collection。

ready 的具体含义由 resource 自己拥有：image 等待成功 decode，font 等待将要渲染的精确 face，texture
等待上传到 presentation 的 graphics context。deadline 后仍在 pending 的 preparation，在平台提供
取消能力时必须由 `dispose` 取消；无论平台是否可取消，迟到的 completion 都不得重新安装已释放状态。
只注册一个不拥有 browser resource 的任意 promise 不满足本合约。

`@onmark/authoring` 提供 `createImageResource({ document, id, source })`
与 `createFontResource({ face, fonts, id })`。image helper 暴露自有 element 供 authored
layout 使用，并以 `decode()` 作为 readiness；font helper 先加载精确 `FontFace`，再把它加入传入的
`FontFaceSet`，dispose 之后迟到的 completion 不会重新加入该 face。

## 确定性规则

presentation 代码必须在 runtime frame clock 下确定。

允许：

- 静态 CSS 和 DOM layout；
- 由 runtime callback 驱动的本地浏览器 effect；
- runtime adapter 拥有的有界 resource readiness；
- 输出只依赖已求解 plan facts 和 bundled assets 的语义 class 或自定义元素。

不允许：

- 用 `Date.now()`、墙钟 timer、随机值或环境动画进度决定像素；
- 用浏览器 media clock 决定捕获哪一帧；
- 让网络请求或可变外部状态参与输出；
- 在 TypeScript 里重写 cue、delay、duration 或 partition 逻辑；
- 无界等待、队列或 retained buffer。

Gate 五只接纳由精确 `RuntimeFrame` 驱动、playhead 已暂停的动画。首个 conformance matrix 通过标准 frame-effect lifecycle 覆盖 WAAPI、GSAP 与 Three.js，但不会让这些库成为 runtime dependency。依赖加载时刻的静态 CSS transition、free-running library ticker 和 ambient `requestAnimationFrame` progress 仍不属于确定性合约。通过 lifecycle 不等于 bundle 获得 random access；capability metadata 只会与 partitioning proof 一起落地。

## 失败与清理

预期浏览器失败通过 runtime protocol
failure 返回。自定义 adapter 如果能识别操作或 readiness 失败，应抛出
`RuntimeAdapterError`；readiness timeout 应携带有界的 pending resource 名称。

dispose 是终止相位。presentation 可以报告清理失败，但不能让半清理状态重新服务。浏览器 API 允许时，资源清理应保持幂等。

## 非目标

Gate 一不提供 presentation dev server、watch mode、plugin API、component
registry、由 screenplay 选择的组件或 props、跨场景 persist、自由
`begin/end/until` 时间表达式或 browser-side render
planning。这些能力必须先有明确语言语义、runtime 合约和评测证据，才能成为公开契约。
