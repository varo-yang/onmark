# Onmark Presentation Contract

> 状态：当前 browser authoring 合约已覆盖进行中的 Gate 八及分布式增量渲染。

`film.html` 是完整的作者入口。Onmark custom element 拥有结构、ID、cue、素材引用和时间关系；
普通 HTML 与 inline CSS 拥有 presentation。可选的
`<script type="module" data-om-motion>` 导出 browser effect。没有平行 stylesheet、
motion 文件、generated DOM、template 或 custom-entry mode。

browser 收到投影掉 compiler-only fact 后的 presentation-owned DOM。Bundler 会移除已由
Timeline IR 或 Browser Plan 持有的 cue、audio、素材引用与 timing 事实；ID、class、普通
属性、嵌套标记、inline style 与作者文本保持为精确 browser 输入。因此只修改既有 music、
sfx、vo、cue declaration 或 compiler timing attribute 的值或内容不会改变 presentation
identity；source restructuring 仍可能改变 presentation whitespace。Rust 仍然拥有所有
interval；HTML、CSS、Canvas、WebGL、GSAP、Three.js 等 browser library 可以渲染已求解
事实，但不能解析 cue、推导 shot 时长、规划分片或选择帧区间。

## 最小入口

静态 film 不需要作者 JavaScript：

```bash
onmark render film.html
```

创作过程中可以在不编码完整影片的前提下捕获一张精确 production frame：

```bash
onmark snapshot film.html --frame 42
```

需要做一轮有界的全片审阅时，可以复用精确 production region 并生成静态 contact
sheet：

```bash
onmark review film.html
```

这份 report 不是 preview runtime。它的 lossless frame、source span、已求解 timing
provenance 与 region identity 都来自完整和增量渲染使用的同一份 artifact。

CSS 直接写在普通 HTML 中：

```html
<style>
  .headline {
    color: white;
    font: 700 8vw/1 sans-serif;
  }
</style>

<om-film>
  <om-scene>
    <om-shot duration="3s">
      <om-title class="headline">Native HTML. Exact time.</om-title>
    </om-shot>
  </om-scene>
</om-film>

<script type="module" data-om-motion>
  import { gsapMotion } from "onmark/motion/gsap";

  export const motion = gsapMotion({
    title({ element, timeline }) {
      timeline.from(element, { opacity: 0, y: 40, duration: 0.4 });
    },
  });
</script>
```

`gsapMotion` 接受一个语义 motion definition。`title` 等具名 handler 面向该类型的
所有 target，`selectors` 下的条目面向匹配 authored ID 或 class 的 target。kind handler
先于命中的 selector 执行，所有 handler 写入同一条 target-owned paused timeline，作者
不需要再写 element-ID switch。

GSAP 不是必需依赖。原生 HTML、SVG、Canvas 与 WebGL effect 可以直接消费精确局部帧：

```html
<script type="module" data-om-motion>
  import { frameMotion, interpolate, spring } from "onmark/authoring";

  export const motion = frameMotion({
    title(context) {
      const progress = spring(context, { damping: 18, stiffness: 160 });
      const y = interpolate(progress, [0, 1], [40, 0]);
      context.element.style.opacity = String(
        interpolate(progress, [0, 1], [0, 1]),
      );
      context.element.style.transform = `translateY(${y}px)`;
    },
  });
</script>
```

每个 `frameMotion` sample 包含 authored element、从零开始的 `localFrame`、target 的
`durationFrames`、有理 `frameRate` 与 `progress`。progress 是与 solved interval 一致的
半开比值 `localFrame / durationFrames`：第一帧为零，exclusive end 到达时元素已不再
active。每次 sample 都从本次 runtime frame 独立推导，因此倒序、重复与分片求值不依赖
此前调用。

`interpolate(...)` 映射分段数值区间，默认 clamp 两端，只有显式请求 `extend` 才会外推。
`easing` 集合提供无状态 linear 与 cubic 曲线。`spring(context, options)` 直接从
`localFrame` 和有理帧率求解阻尼弹簧方程，不逐帧推进，也不选择 target 时长。它们只生成
视觉值，不构成第二套 scheduler。

Onmark 当前不把 `Element.animate()` 包装成 random-access adapter。一次 wrapper 实验发现：
即使 pause 或把 playback rate 固定为零，在同一份 locked Chrome environment 中，
whole-film 与 region-projected capture 仍出现了不确定的文字像素。底层 paused-WAAPI
实验仍证明了同一 document scope 内的重复与乱序 seek；它没有证明一个 region-safe 的公开
adapter。原生 exact-frame effect 应使用 `frameMotion`，ambient browser animation 仍不属于
确定性 contract。

元素内部动效只能消费该语义 target 自带的 interval。唯一准入的跨 shot 例外是显式
`<om-transition>` boundary：Rust 提供已求解 overlap 与 Render Graph dependency，
presentation code 只获得两个相邻 shot element，并在该 interval 内实现像素。

两个 exact-frame adapter 都暴露一个专用 `transition` handler：

```js
export const motion = gsapMotion({
  transition({ incomingElement, outgoingElement, timeline }) {
    timeline
      .to(outgoingElement, { opacity: 0, duration: 0.5 }, 0)
      .from(incomingElement, { opacity: 0, duration: 0.5 }, 0);
  },
});
```

`frameMotion` 的 transition `progress` 使用精确闭合端点：overlap 第一帧为零，最后一帧
为一；这与普通 element 的半开 local progress 有意不同。GSAP adapter 同样会在最后一帧
精确渲染 timeline end。若 overlap 只有一帧，两个 adapter 都把这个唯一 sample 渲染为
终态。两个 adapter 都不选择 transition duration、adjacency 或 partition scope。

bundler 提取这一个可选 module、编译其 import，并在 browser body 末尾安装 generated
runtime script。infrastructure 始终位于 semantic shot 之外，因此 region projection
不会误删它。projection 保留 presentation-owned HTML，同时移除已由 Timeline IR 或 Browser
Plan 表示的 compiler-only cue、audio、素材引用和 timing fact。作者不构造 runtime
adapter、不注册 global timeline、也不拥有
infrastructure cleanup。产物仍是不可变 browser artifact，其 capability 与 identity 由
Rust-owned manifest fact 决定。

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
会绑定 plan 中的每个 video、overlay 与 transition node。导入字幕是 caption role 的
overlay，与其他 overlay 共用已求解 visibility path，不另造 browser timing
engine。当前 region 内的 node 在其 solved interval 使其可见之前不进入 layout 与
compositor；region 外的 semantic sibling 根本不进入 document，因此 selector 与 authored
effect 无法观察它们。`Prepare` 之后，native
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

| Owner                   | Owns                                                                |
| ----------------------- | ------------------------------------------------------------------- |
| Screenplay 与导入字幕   | authored 结构、文本、素材引用、cue、局部 delay                      |
| Rust compiler           | parse、normalize、reference resolution、精确求时、Timeline IR       |
| Runtime                 | protocol 状态、frame clock、视频解码 readiness、visibility interval |
| Authored HTML 与 motion | DOM shape、layout、字体与 browser effect                            |
| Renderer                | materialized asset path、Chromium、capture、encoding                |

presentation 收到的 placement 已经包含绝对帧区间。它可以决定 title 长什么样、CTA 放在哪里、video 怎么被 CSS 布局；它不能把 title 提前、延长 overlay、重新解释
`delay`，也不能从 DOM 里重新推导媒体时长。

## Authoring facade

`@onmark/authoring` 把已求解事实绑定到 authored semantic element：

- `createDomPresentationBindings({ document, videoSource, motion? })` 是 bundle
  infrastructure 安装的低层 facade；
- `<om-film>`、`<om-scene>`、`<om-shot>`、`<om-transition>`、`<video>`、
  `<om-title>` 与 `<om-cta>` 始终是作者写下的原始 element；
- bound node 临时携带 `data-om-node`；authored ID 保持普通 HTML ID；
- whole-film plan 对完整 film 使用 dense renderable-semantic preorder；每个 region plan
  则对 selected film、scene、shot 与 content 独立生成 dense preorder；
- authored ID 在 projection 间保持稳定，是 presentation 需要跨 build 语义身份时使用的
  selector；protocol `nodeId` 只是 unit-local binding key，不是持久 whole-film address；
- region 外的 semantic sibling 会被省略而不是仅仅隐藏，因此 selector 无法制造未声明的
  cross-region dependency；
- native composition 拥有 primary video 时，`bindFilm(plan)` 会在任何 content
  binding 之前把该 video 从 dense foreground identity 中投影掉；authored
  `<video>` 保持隐藏，Chromium 不会加载或 seek 它；
- 导入 caption 是 facade 唯一创建的 DOM node，因为它不在 authored document 中；
- runtime 根据已求解 interval 切换 container 与 content visibility，CSS 独占 layout
  与视觉设计。

更精确地说，production adapter 会先调用 `bindFilm(plan)` 建立完整 projection，再绑定
scene、shot container，并在 `load` 时调用 `bindVideo(placement)`、
`bindOverlay(placement)`、`bindTransition(relation)` 与异步的
`bindExtensions(plan)`。extension 返回其待准备 resource 和拥有的精确逐帧 effect。video binding 提供浏览器 element、已 materialize 的 source、visibility effect
和终止性 cleanup；overlay binding 提供 visibility 与终止性 cleanup；transition binding
提供 marker、两个相邻 shot element 与终止性 cleanup，extension 拥有视觉状态而不拥有
timing。compiler-owned
node identity 在每个 projection 内形成独立的 dense unit-local 顺序；跨 projection 的语义身份由
authored ID 承担。每次 `seek` 时，runtime 先隐藏 video，再根据权威 output frame 选择已准入的
source frame、呈现 ready video，最后应用已求解 overlay 的 visibility。
video readiness timeout 以 `video:<nodeId>:loadeddata`、
`video:<nodeId>:seeked` 或 `video:<nodeId>:frame` 指出 unit-local node 与 phase。
binding 拥有效果，不拥有 interval arithmetic。

## Plan facts 与 canonical typed variants

authored HTML 收到的 dynamic fact 是 `Load(plan)` 传入的 Rust-owned
`BrowserPlan`：帧率、完整 solved film interval、evaluation/output interval、semantic
structure 与 ownership、video placement、transition relation、title/CTA/imported caption
overlay placement，以及当前 Render Graph region 所需的 canonical typed variant value。
与 unit 相交的 structure 与 overlay 保留完整 solved interval；`evaluation` 只选择该 unit
执行的 frame，`output` 只选择其发布的 frame。

这些 dynamic fact 还包含当前 Render Graph region 所需的 canonical typed variant
value。语言仍然没有 `presents`、`definePresentation`、任意 props object、source
placeholder substitution、module-owned input schema、global 或 URL parameter。唯一的
author-input surface 是语言规格书定义的封闭 `om-fields` schema，以及
`data-om-text`、`data-om-css` 与 `data-om-show` 三种 literal sink。Rust 用 schema
校验 external JSON 并产出按 name 排序的 value vector；runtime 不接收 untyped input
object，也不解释 source JSON spelling。

production adapter 在 `load` 中、motion prepare 之前应用全部已验证 binding：

- `data-om-text` 通过 `textContent` 写入一个 `text` value；
- `data-om-css` 通过 `style.setProperty("--<field>", canonicalValue)` 写入 `color`
  或 `integer` value；
- `data-om-show` 通过 `hidden` property 写入一个 `boolean` value。

bundler 会从 projected document 移除 `om-fields`/`om-field` declaration，但保留三种
binding attribute；它不会替换 source byte，也不会从 value 生成 executable code。
runtime binding 受已验证 plan 与 document schema 双重约束。target 缺失、同一 target
拼写重复、value kind 不兼容或 plan 携带当前 region 不需要的 field 都属于 protocol
failure，不能 best-effort 忽略。

binding 是 presentation state，不是 timing input。它们在 extension resource discovery
和 motion prepare 之前完成，因此后续 phase 只观察一份稳定 DOM。`seek` 不会重复应用
binding；一份 Render Unit 只拥有一个 immutable variant。variant 改变会产生不同 Browser
Plan，并只在 field 实际被消费的 region 形成不同 frame-artifact identity。

field dependency 沿用 document projection：shot-owned binding 只进入包含该 shot 的
region；scene-shell binding 进入保留该 scene 的每个 region；film-shell binding 进入全部
region；transition binding 只进入同时保留两个相邻 shot 的 region。Timeline IR 拥有这些
scope，Render Graph 负责解析，Browser Plan 只携带 selected region 的 value。TypeScript
不得从 projected DOM 反推 dependency。

既有内建 component fact 仍保持封闭：`nodeId` 是 dense unit-local binding key，可选
`authoredId` 用于跨 projection 的 semantic selection，`kind` 只选择 title、CTA 或
caption，`text` 是 component 携带的 authored fallback。typed binding 可以替换
presentation 的 literal DOM text，但不能重新解释 screenplay structure 或 solved
interval。

stylesheet rule 与 inline module 静态 import 的值仍是 presentation code，不是 variant
value。presentation code 不得从可变 side channel 读取作者意图。

## Temporal capability

Bundle contract 携带由 `@onmark/runtime` 拥有的封闭
`PresentationTemporalCapability`。当前只接纳 `sequential` 与
`randomAccess`；`warmup(n)` 及更宽的依赖分类仍只是架构设想。它不是用户 CLI 选项。
production authored-HTML surface 已准入 `randomAccess`：唯一动态输入是 immutable
Browser Plan fact、已准备 resource 与精确请求的 `RuntimeFrame`；motion contract 要求
effect 为该 frame 直接设置状态，不能依赖之前调用逐步推进。ambient clock、隐藏 queue 与
stateful frame accumulation 都违反 contract。未知的未来 browser component 仍为
`sequential`，直到独立 conformance 接纳更强行为。低层 bundler 因为还负责构造已证明的
conformance artifact，仍要求显式 capability。

低层 `FrameEffect` 与 `PresentationResource` boundary 由 `@onmark/runtime` 拥有。
`@onmark/authoring` 暴露 vendor-neutral 的 `PresentationExtension` contract；
`frameMotion(...)` 拥有 native procedural exact-frame effect。只有组合彼此独立的多个
adapter 时才使用 `combineMotion(...)`，并按声明顺序执行。
`onmark/motion/gsap` 是由内部依赖包承载的可选 adapter：它把 semantic hook 转成 paused
GSAP timeline，但不让 GSAP 进入 runtime 或 authoring。Three.js、Lottie 或应用本地引擎都可
实现同一 contract；bundler 与 runtime 不包含 vendor branch。每个 GSAP hook 只收到 semantic element、compiler-owned
duration，以及一条由 adapter 拥有、以局部秒计量的 paused timeline；transition hook
还会收到两个相邻 shot element。adapter 在 seek 时
抑制 callback，而且即使请求的 local time 与当前 playhead 相同也会强制 render，并拥有
terminal cleanup。这样零时刻 `.set()` 与重复的 exact-frame 请求不会被 GSAP 当作 no-op。
每次 `Seek(frame)` 中，effect 会在 solved video 与 overlay placement
之后按声明顺序 apply，所有返回 promise 都必须在 `FrameStaged(frame)`
前完成。effect 只获得精确 immutable `RuntimeFrame`，不会得到 scheduler 或 mutable
timeline。effect 按所有权逆序释放；单个 cleanup 失败后仍会尝试 dispose 全部 effect。

实现这条 lifecycle 并不会让任意 component 自动取得 random access。production adapter
另有 WAAPI、GSAP 与 Three.js 乱序 playhead conformance；一条 real-process GSAP
conformance 会把跨 scene 的 timeline 分别作为 whole-film 与两个独立 unit 渲染，再比较完整
raw-RGBA sequence。该证明依赖 Browser Plan 跨 evaluation window 保留完整 solved interval。
未来 adapter 也必须提供同等级证据。能力是 immutable build metadata，不从 source token 或
screenplay spelling 猜测。当前 bundle manifest 把它纳入 canonical identity，Rust 在 Render
Graph 分片前消费它。

## Document scope 与 region projection

`PresentationDocumentScope` 记录 immutable bundle 的 DOM 范围：

- `wholeFilm` 包含完整 presentation-owned film，用于 whole-film execution 与 conformance；
- `renderRegion` 包含一个 selected shot、其 owning scene/film shell，以及
  presentation-global style、motion 与 imported resource byte。

production bundler 从一次 compilation 同时产出两者。generated module 与 resource 会
hard-link 到 region root；每个 region 拥有独立 projected `index.html`、manifest 与
`bundleId`。因此局部 shot edit 只改变该 region，除非它同时改变 presentation-global byte
或 Rust-owned dependency fact。`documentScope` 与 temporal capability 相互独立：前者说明
DOM 中有什么，后者说明 artifact 能否独立求值任意请求 frame。

shot 内的 `<style>` 是该 shot 的局部输入。scene 内、shot 外的 style 属于该 scene 的所有
region；位于所有 scene 之外的 film/document rule 属于全部 region。presentation code
必须把 rule 放在覆盖全部 consumer 的最窄 semantic owner 下，也不得通过 relational
selector 等方式把已省略的 semantic sibling 当作未声明的共享状态。

## Visual capability

`PresentationVisualCapability` 声明 Chromium 可以拥有哪些像素。它是 build metadata，
不是 screenplay spelling，也绝不从 authored browser code 猜测。authored HTML
可以且只能声明一次：

```html
<meta name="onmark:visual-capability" content="separableBackdrop" />
```

没有声明时使用 `browserComposite`。底层 bundler 若同时收到显式配置，该配置必须与
authored value 一致；冲突是错误，不是覆盖。

- `browserComposite` 表示 Chromium 拥有包括主视频在内的完整画面，是未知
  presentation code 的保守能力；
- `separableBackdrop` 表示 Chromium 拥有位于一个或多个 native video rectangle
  下面的 browser backdrop。renderer 可以测量这些 rectangle、省略 browser video
  pixel，再把 native media 放到捕获的 backdrop 上方；
- `separableOverlay` 表示 Chromium 只产出与主视频像素无关的透明前景，native
  execution 可以先解码并安放主视频，再以 source-over 合成该前景。

声明 `separableOverlay` 的 presentation 在 browser video placement 被移除后仍必须
保持正确。它可以使用 solved interval、overlay fact、精确 frame identity 与 immutable
visual resource；不得把 video 采样进 Canvas/WebGL，不得读取 media pixel，不得使用依赖
背景的 filter 或 blend mode，也不得让前景像素以其他方式依赖下面的主画面。能力由
conformance 接纳，不能因为 source scan 暂时没找到禁用 token 就获得信任。

`separableBackdrop` 同样是一项强 author contract。browser-owned pixel 必须始终位于
每个声明的 native video 下面；video rectangle、`object-fit` 与 `object-position`
在整个 unit 内必须保持不变。presentation motion 不得通过修改 video 或 ancestor
改变这些 geometry。当前封闭子集最多接纳 16 个 video、device pixel ratio 1、正的
整数 viewport rectangle、`fill`/`contain`/`cover`，以及两个精确 percentage
position。video 的 border 与 padding 必须为零，也不得使用 radius、transform、
filter、clip path、opacity 或 blend mode。导出的 source crop 与 destination scale
也必须为整数且不越界。native video rectangle 只有在 solved interval 不重叠时才可
占用同一像素区域。

capture 之前，runtime 会以 `layoutOnly` 模式加载同一份 immutable Browser Plan。
它逐个显示 shot structure，在保留 video layout box 的同时隐藏其像素，并通过带版本的
browser protocol 返回 node-keyed geometry。Rust 校验数量、顺序、identity、边界、
精确 crop/scale 算术与时空重叠，然后把结果冻结成当前 capture transaction 的
`BackdropLayoutPlan`。随后 Chromium 以 media omitted 模式加载，由一条 persistent
`FFmpeg` process 把 native media 放到 browser backdrop 上方。声明不成立时通过 typed
plan/runtime error 失败，execution 绝不切换到另一条像素路径兜底。

browser evidence 保证 CSS geometry 精确，并不意味着两个独立 video renderer 会逐像素
相同。声明 `separableBackdrop` 后，video pixel 由 locked native decoder、color
conversion 与 scaler 负责。若 presentation 需要 Chromium 的确切 media rasterization，
应继续使用 `browserComposite`。Onmark 证明所选 native path 在 whole、partitioned、
local 与 worker execution 间的 raw-RGBA 相等，而不宣称两种不同 pixel ownership
contract 彼此相等。

`separableOverlay` path 刻意比它的 presentation promise 更窄：必须恰好有一个覆盖完整
published interval 的主视频，冻结的 source dimensions 必须与 output profile 完全一致，
并且完整 color tuple 必须属于已准入的 BT.709 limited-range profile。这些检查避免 Rust
重造 CSS layout。capability 是许可而不是执行命令：planning 只在这些事实证明 native
profile 时选择 `separableOverlay`，否则把 `browserComposite` 明确写进 execution plan。
计划一旦生成便不可变；worker 启动后绝不换路，transported plan 若超出 capability 仍会
校验失败。

当前 Bundle Manifest 把 temporal、visual capability 与下面的 frame behavior 都纳入
canonical identity。bundle 是可重建产物而非 authored data；reader 只接受当前版本，
旧 bundle 直接重建。

对 `separableBackdrop`，artifact identity 会绑定布局证据的全部推导输入：
content-addressed bundle byte、Browser Plan、render profile、预期 native media fact
与 locked capture environment。browser response 不会被重复塞进 portable Render Unit
或 cache key；否则每次 cache lookup 前都必须先启动 Chromium。local 与 distributed
execution 会从同一 Render Unit 执行同一项 bounded preflight，再在 conformance 中比较
canonical raw-RGBA output。

## Frame behavior

`PresentationFrameBehavior` 声明 browser-owned pixels 是否会在 Rust-owned placement
boundary 之间变化：

- `perFrame` 是保守值，Chromium 可能需要求值并捕获每个 authored frame；
- `placementBounded` 证明 visible fact 不跨 video、overlay 或 structural placement
  boundary 时，browser pixels 保持完全相同。

该声明与 visual separability 相互独立。CLI 保守声明 `perFrame`。更强行为必须同时具备
`randomAccess`：只有后续 boundary frame 可以直接求值时，native 才能跳过中间的
`Seek` 与 `Confirm`。

capability 仍是许可，不是 cache 指令。planning 只会在 Chromium 不拥有 video pixels
时记录 `placementBounded` capture。含 browser video 的 browser-composite unit 仍是
`everyFrame`；native-video `separableOverlay` unit 与纯静态 browser unit 才能使用更强
cadence。native 捕获首个 output frame 与每个已求解 boundary，然后在中间 output frame
之间共享同一份 encoded PNG payload，但仍逐帧写入 encoder 或 worker artifact。

frame behavior 是进入 `bundleId` 的 immutable build metadata。它绝不从 source token、
观测到的 pixel equality、compositor damage 或 screenplay spelling 推断。worker request
携带已准入 cadence；任何与 bundle declaration 或 materialized visual plan 不一致的值都会被拒绝。

## 素材

浏览器只看 unit root 下已 materialize 的素材。video placement 使用：

```ts
materializedVideoSource(placement);
```

这个 helper 从 Rust-owned browser plan 里的 frozen asset identity 推导
`./assets/sha256/<digest>`。presentation 不应该拼 native
path、读取源码文件或假设 working
directory。renderer 会在浏览器看到素材前验证字节。

原生 `<img src>` 可以引用本地 AVIF、GIF、JPEG、PNG、SVG 或 WebP。bundler 会冻结这些
bytes，把 URL 改写为不透明的 `resources/` 路径，并让每个 shot projection 只保留自己实际
引用的 image；runtime 会自动等待这些 authored image 完成 decode。remote URL 与 `srcset`
会被拒绝，不允许逃出 frozen resource boundary。同一边界也会拒绝会自行推进的 image bytes：
multi-frame GIF、APNG、animated WebP/AVIF，以及带 animation、script、event 或嵌套 image
能力的 SVG 都不能引入 wall-clock playhead；动画必须由 Onmark frame effect 驱动。这条准入同时
覆盖本地 `src`、data URL 与 image import。

inline motion module 还可以 import 上述 image 格式、OTF、TTF、WOFF、WOFF2，或引用这些格式的
本地 CSS module。inline `<style>` 中的裸相对 URL 仍不属于 import boundary。bundle 会证明
imported resource 的 byte identity；动态构造的 browser resource 仍必须显式注册：

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
与 `createFontResource({ face, fonts, id })`。image helper 暴露自有 element 供动态 layout
使用，并以 `decode()` 作为 readiness；静态原生 `<img>` 会自动获得同一生命周期。font helper
先加载精确 `FontFace`，再把它加入传入的 `FontFaceSet`，dispose 之后迟到的 completion
不会重新加入该 face。

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

当前动画合约只接纳由精确 `RuntimeFrame` 驱动、playhead 已暂停的动画。准入 conformance matrix 通过标准 frame-effect lifecycle 覆盖 WAAPI、GSAP 与 Three.js，但不会让这些库成为 runtime dependency。依赖加载时刻的静态 CSS transition、free-running library ticker 和 ambient `requestAnimationFrame` progress 仍不属于确定性合约。通过 lifecycle 不等于 bundle 获得 random access；capability metadata 只会与 partitioning proof 一起落地。

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

当前合约不提供 presentation dev server、watch mode、plugin API、component
registry、由 screenplay 选择的组件或 props、跨场景 persist、自由
`begin/end/until` 时间表达式或 browser-side render
planning。这些能力必须先有明确语言语义、runtime 合约和评测证据，才能成为公开契约。
