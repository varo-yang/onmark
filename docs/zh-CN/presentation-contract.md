# Onmark Presentation Contract

> 状态：已覆盖 Gate 一至 Gate 七。

`film.onmark` 是唯一必需的作者入口，拥有结构、内容、ID、cue、素材引用和时间关系。
普通 render path 会把已求解事实投影成 semantic DOM node，并在存在时 bundle 同名的
stylesheet `film.css` 与 motion module `film.motion.ts`。这条投影自身不提供颜色、layout、
motion、theme 或 component template。

需要任意 DOM、Canvas、WebGL、resource lifecycle 或 library adapter 的 film，可以用
`--presentation` 选择自定义 TypeScript entry。高级路径保留完整 presentation boundary，
而不是以削减能力换取简单。Rust 仍然拥有所有 interval；CSS 与 TypeScript 可以渲染已
求解事实，但不能解析 cue、推导 shot 时长、规划分片或选择帧区间。

## 最小入口

普通路径不需要作者 JavaScript：

```bash
onmark render film.onmark
```

若 screenplay 旁存在 `film.css`，bundler 会将其纳入 artifact。CSS 可以自由设置
`.onmark-film`、`.onmark-scene`、`.onmark-shot`、`.onmark-video`、`.onmark-title`、
`.onmark-call-to-action` 与 `.onmark-caption`；缺失时保留 browser 默认值。semantic stylesheet
path 即使同时存在 motion，也不准入需要观测 readiness 的本地字体或图像；这类资源需要 custom
presentation 显式注册 `PresentationResource`。若存在
`film.motion.ts`，它只导出一个 `motion` value：

```ts
import { gsapMotion } from "onmark/motion/gsap";

export const motion = gsapMotion({
  title({ element, timeline }) {
    timeline.from(element, { opacity: 0, y: 40, duration: 0.4 });
  },
});
```

`gsapMotion` 接受一个语义 motion definition。`title` 等具名 handler 面向该类型的
所有 target，`selectors` 下的条目面向匹配 authored ID 或 class 的 target。kind handler
先于命中的 selector 执行，所有 handler 写入同一条 target-owned paused timeline，作者
不需要再写 element-ID switch。

元素内部动效只能消费该语义 target 自带的 interval。当前不准入跨 shot
转场；它的窗口与相邻依赖必须先成为 Rust-owned Render Graph fact，不能在
TypeScript 中再造一套 timing policy。

bundler 编译一条固定基础设施 entry，导入这些可选文件并安装 semantic DOM runtime。作者
不构造 runtime adapter、不注册 global timeline、也不拥有 cleanup。产物仍是不可变
browser artifact，其 identity 仍由 Rust-owned manifest fact 决定。

高级 presentation 必须显式选择：

```bash
onmark render film.onmark --presentation presentation.ts
```

自定义代码直接把 `PresentationBindings` 与 `@onmark/runtime` 组合。便利接线只由
bundler 生成的 neutral entry 拥有，authoring 仍然只依赖 runtime types。

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
      -> Confirm(frame) -> FrameReady(frame)
      -> [native placement-boundary reconciliation capture])*
  -> Dispose
```

这个拆分来自 Chromium decoded-media 的真实约束：`requestVideoFrameCallback`
必须在它要观察的 compositor frame 之前注册；但在 CDP BeginFrameControl
target 上，如果先等 callback 再发送 `BeginFrame`，两边会形成死锁。因此
`FrameStaged(frame)` 只表示 browser
state 已能进入 compositor。native 随后为每个 output
frame behavior 选中的 frame 发送一次正常的、同时 commit frame state 与 capture PNG 的
`HeadlessExperimental.beginFrame`。`perFrame` 选择所有 output frame；
`placementBounded` 只选择首个 output frame 与每个已求解 placement boundary，
中间 output frame 会复用上一份精确 PNG，不再发起 runtime transaction。在 video 或
overlay boundary，native 会先在当前
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

| Owner                                    | Owns                                                                |
| ---------------------------------------- | ------------------------------------------------------------------- |
| Screenplay 与导入字幕                    | authored 结构、文本、素材引用、cue、局部 delay                      |
| Rust compiler                            | parse、normalize、reference resolution、精确求时、Timeline IR       |
| Runtime                                  | protocol 状态、frame clock、视频解码 readiness、visibility interval |
| Default authoring 或 custom presentation | DOM shape 与 browser effect                                         |
| Presentation CSS                         | layout、字体与视觉风格                                              |
| Renderer                                 | materialized asset path、Chromium、capture、encoding                |

presentation 收到的 placement 已经包含绝对帧区间。它可以决定 title 长什么样、CTA 放在哪里、video 怎么被 CSS 布局；它不能把 title 提前、延长 overlay、重新解释
`delay`，也不能从 DOM 里重新推导媒体时长。

## Authoring facade

`@onmark/authoring` 提供默认语义 DOM bindings：

- `createDomPresentationBindings({ document, videoSource, motion? })` 是固定 entry 与
  custom adapter 使用的低层 facade；
- film、scene 与 shot fact 形成 nested `<main>`、`<section>` 与 `<article>`；video 与
  authored overlay 进入 owning shot，导入 caption 进入 film root；
- 每个 node 都携带 `data-onmark-node`；authored ID 同时成为 DOM ID 与
  `data-onmark-id`；
- runtime 根据已求解 interval 切换 container 与 content visibility，CSS 独占 layout
  与视觉设计。

默认 facade 刻意很小。presentation 可以直接实现 `PresentationBindings`
来支持 Canvas、WebGL 或自定义 DOM，但规则不变：binding 创建浏览器资源，`setVisible`
应用可见性，`dispose` 终止性释放资源。

更精确地说，production adapter 会先绑定 film、scene、shot container，再在 `load` 时
调用 `bindVideo(placement)`、`bindOverlay(placement)` 与异步的
`bindExtensions(plan)`。extension 返回其待准备 resource 和拥有的精确逐帧 effect。video binding 提供浏览器 element、已 materialize 的 source、visibility effect
和终止性 cleanup；overlay binding 提供 visibility 与终止性 cleanup。compiler-owned
node identity 在更早元素未进入某个 partition 时仍保持不变。每次 `seek` 时，runtime 先隐藏
video，再根据权威 output frame 选择已准入的 source frame、呈现 ready video，最后应用已求解 overlay 的 visibility。
binding 拥有效果，不拥有 interval arithmetic。

## Plan facts、组件选择与 props

当前语言**没有** `presents`、`definePresentation`，也没有 screenplay 到 presentation
的 props 通道。`onmark render` 默认使用 semantic DOM projection，只有
`--presentation` 才会显式选择自定义代码。两条路径唯一收到的动态事实，都是 `Load(plan)`
传入的 Rust-owned `BrowserPlan`：帧率、evaluation/output interval、semantic structure
与 ownership、video placement，以及 title、CTA 或导入 caption 的 overlay placement。
stylesheet rule 与自定义 TypeScript 静态 import 的值都是 presentation code，不是
screenplay props。

这些既有 fact 构成封闭的内建 component contract：`nodeId` 是稳定的投影身份，可选
`authoredId` 用于 semantic selection，`kind` 只选择 title、CTA 或 caption，`text` 是该 component
唯一的 authored property。这不会创建通用 props 通道，也不允许 presentation
重新解释 screenplay 结构。

这项缺失是有意边界，不是未写下来的约定。未来的 presentation selection 或 props
feature 必须一起定义：screenplay spelling、带类型的 schema/default、canonical
wire encoding、source span 与 diagnostic、bundle/cache identity，以及与 temporal
capability declaration 的关系；它还必须具备受控 language
evaluation 证据。在这些工作完成前，presentation 不得从 global、URL
parameter、可变 side channel 或自造的 `presents` attribute 读取作者意图。

## Temporal capability

Bundle contract 携带由 `@onmark/runtime` 拥有的封闭
`PresentationTemporalCapability`。当前只接纳 `sequential` 与
`randomAccess`；`warmup(n)` 及更宽的依赖分类仍只是架构设想。它不是用户 CLI
选项：只有无 authored CSS、无 motion 的内置 semantic DOM 由 Onmark 准入为 random access；
stylesheet、authored motion 与 custom presentation 保守为 sequential；只有底层
conformance bundler 为已证明产物显式传值。

低层 `FrameEffect` 与 `PresentationResource` boundary 由 `@onmark/runtime` 拥有。
`@onmark/authoring` 暴露 vendor-neutral 的 `PresentationExtension` contract。单个 adapter
直接导出；只有组合彼此独立的多个 adapter 时才使用 `combineMotion(...)`，并按声明顺序执行。
`onmark/motion/gsap` 是由内部依赖包承载的可选 adapter：它把 semantic hook 转成 paused
GSAP timeline，但不让 GSAP 进入 runtime 或 authoring。Three.js、Lottie 或应用本地引擎都可
实现同一 contract；bundler 与 runtime 不包含 vendor branch。每个 GSAP hook 只收到 semantic element、compiler-owned
duration，以及一条由 adapter 拥有、以局部秒计量的 paused timeline；adapter 在 seek 时
抑制 callback 并拥有 terminal cleanup。每次 `Seek(frame)` 中，effect 会在 solved video 与 overlay placement
之后按声明顺序 apply，所有返回 promise 都必须在 `FrameStaged(frame)`
前完成。effect 只获得精确 immutable `RuntimeFrame`，不会得到 scheduler 或 mutable
timeline。effect 按所有权逆序释放；单个 cleanup 失败后仍会尝试 dispose 全部 effect。

这条 lifecycle 本身不是 random-access 声明。只有 conformance 证明任意请求帧只依赖
immutable input 与该精确帧后，adapter 才能取得更强能力。能力是 immutable build
metadata，不从 source 或 screenplay spelling 猜测。当前 bundle manifest 把它纳入
canonical identity，Rust 在 Render Graph 分片前消费它。

## Visual capability

`PresentationVisualCapability` 声明 Chromium 可以拥有哪些像素。它是 build metadata，
不是 screenplay spelling，也绝不从未知 presentation code 猜测。CLI 自己拥有、且没有
authored CSS 或 motion 的 neutral semantic DOM 声明 `separableOverlay`；authored CSS、
motion 与 custom entry 仍保持 `browserComposite`。底层 conformance bundler 只为已证明
产物显式传值。

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
layout。capability 是许可而不是执行命令：planning 只在这些事实证明 native profile 时选择
`separableOverlay`，否则把 `browserComposite` 明确写进 execution plan。计划一旦生成便不可
变；worker 启动后绝不换路，transported plan 若超出 capability 仍会校验失败。

当前 Bundle Manifest 把 temporal、visual capability 与下面的 frame behavior 都纳入
canonical identity。bundle 是可重建产物而非 authored data；reader 只接受当前版本，
旧 bundle 直接重建。

## Frame behavior

`PresentationFrameBehavior` 声明 browser-owned pixels 是否会在 Rust-owned placement
boundary 之间变化：

- `perFrame` 是保守值，Chromium 可能需要求值并捕获每个 authored frame；
- `placementBounded` 证明 visible fact 不跨 video、overlay 或 structural placement
  boundary 时，browser pixels 保持完全相同。

该声明与 visual separability 相互独立。CLI 自己拥有、且没有 authored CSS 或 motion
的 neutral semantic DOM 声明 `placementBounded`；authored CSS、motion 与 custom
presentation 都保持 `perFrame`。更强行为必须同时具备 `randomAccess`：只有后续 boundary
frame 可以直接求值时，native 才能跳过中间的 `Seek` 与 `Confirm`。

capability 仍是许可，不是 cache 指令。planning 只会在 Chromium 不拥有 video pixels
时记录 `placementBounded` capture。含 browser video 的 browser-composite unit 仍是
`everyFrame`；native-video `separableOverlay` unit 与纯静态 browser unit 才能使用更强
cadence。native 捕获首个 output frame 与每个已求解 boundary，然后在中间 output frame
之间共享同一份 encoded PNG payload，但仍逐帧写入 encoder 或 worker artifact。

frame behavior 是进入 `bundleId` 的 immutable build metadata。它绝不从 source token、
观测到的 pixel equality、compositor damage 或 screenplay spelling 推断。worker request
携带已准入 cadence；任何与 bundle declaration 或 materialized visual plan 不一致的值都会被拒绝。

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

extension 的 `bind()` 结果最多包含 256 个 resource；其 `kind:id` identity 必须唯一、非空、去除首尾空白且长度
有界。`Prepare` 会在 adapter 的共享 readiness deadline 下并发启动全部 resource、等待全部有界结果，
并把所有超时 identity 报成 `<kind>:<id>:prepare`。未类型化的 preparation failure 也会被收敛到同一
identity。terminal disposal 按声明顺序等待所有 resource；即使一个 cleanup 失败，也不会跳过后续
resource，并只保留第一个 failure。
任何失败的 `Prepare` 都会让 runtime session 与 presentation adapter 进入终止状态，此后只允许
`Dispose`。这样无法取消、迟到的 resource preparation 就不会与第二次 preparation 重叠。
factory 在返回前仍拥有自己创建的 effect；如果构造到一半抛错，它必须自行释放这些 partial
effect。runtime 只接管已经返回的 collection。
同一结果最多可包含 10,000 个精确逐帧 effect；超过上限会拒绝该 presentation，并释放两个已返回的 collection。

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
`Load` 一旦进入作者 binding，后续任何 load、prepare、seek 或 confirmation failure 都会终止该 session，
此后只允许 `Dispose`。在作者代码运行前被 wire validation 拒绝的 request 不会消耗 empty session。

native browser boundary 同样会执行禁止网络的规则：只允许 private Unit Root 下的 canonical file
以及内存 `data:`、`blob:` URL；HTTP、WebSocket 与 root 外 file path 都会被 CDP 拦截。

## 非目标

Gate 一不提供 presentation dev server、watch mode、plugin API、component
registry、由 screenplay 选择的组件或 props、跨场景 persist、自由
`begin/end/until` 时间表达式或 browser-side render
planning。这些能力必须先有明确语言语义、runtime 合约和评测证据，才能成为公开契约。
