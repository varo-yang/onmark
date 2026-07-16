# Onmark Presentation Contract

> 状态：Gate 一浏览器 authoring 合约；Gate 二原样复用。

Gate 一使用两个作者文件：

- `film.onmark` 拥有剧本事实：结构、内容、ID、cue、素材引用和时间关系；
- `presentation.ts` 拥有浏览器 effect：DOM、CSS、layout，以及安装 runtime host。

这个拆分是有意的。剧本保持可朗读、可编译、由 Rust 拥有语义；
presentation 获得正常 TypeScript 工程能力，但不能变成第二套求时语言。Rust
仍然拥有所有 interval。TypeScript 可以渲染已求解事实，但不能解析 cue、推导
shot 时长、规划分片或选择帧区间。

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
packages、生成一个不可变 browser artifact，并用 Rust-owned bundle manifest
记录它。

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

`load` 收到已接受 `BrowserPlan` 的递归冻结快照。它可以创建资源，但不能保留一份可变的
author-owned plan。`prepare` 恰好在 `plan.evaluation.start` 运行一次，且只能在该帧所需资源
稳定后 resolve。`seek` 只会在 prepare 成功后运行；它应用请求的 DOM 状态、预先注册
decoded-media observer，并在媒体完成 seek 后 resolve，但不能等待 compositor presentation。
`confirm` 在 native capture 后运行，只有 browser media 证明该 capture 呈现了 staged source
frame 才能 resolve。即使 cleanup 报错，`dispose` 也是终止相位。

`seek` 不接受自由时间 `t`，而是接收 `RuntimeFrame`：

```ts
interface RuntimeFrame {
  readonly index: number;
  readonly timeSeconds: number;
}
```

`index` 是 native executor 选择的绝对、精确帧身份。`timeSeconds` 只是经 Rust-owned
有理帧率推导出来、供浏览器 API 使用的投影；它不能成为另一套调度时钟或时间决策来源。

## Runtime 握手

presentation 必须用 `installRuntimeHost` 安装一个 runtime host。navigation 与 host discovery
完成后，native renderer 会先在 compositor time zero 发送一次关闭 display updates 的 hidden
BeginFrame，以初始化 Chromium 可复制的 surface；它发生在 `Load` 之前，因此不会 evaluate
authored plan state。真实 capture 使用另一个固定的正 compositor baseline。随后才发送版本化
browser protocol：

```text
Load(plan) -> Prepare(evaluationStart)
  -> (Seek(frame) -> FrameStaged(frame)
      -> native BeginFrame capture
      -> Confirm(frame) -> FrameReady(frame))*
  -> Dispose
```

这个拆分来自 Chromium decoded-media 的真实约束：`requestVideoFrameCallback` 必须在它要
观察的 compositor frame 之前注册；但在 CDP BeginFrameControl target 上，如果先等 callback
再发送 `BeginFrame`，两边会形成死锁。因此 `FrameStaged(frame)` 只表示 browser state 已能
进入 compositor。native 随后为每个 output frame 只发送一次同时 commit 与 capture PNG 的
`HeadlessExperimental.beginFrame`。no-damage response 复用上一张 PNG；只有空的首帧 capture
会获得一次有界的亚毫秒重试。`Confirm(frame)` 让预先注册的 callback 完成；
`FrameReady(frame)` 表示刚才 capture 的帧已经通过精确 decoded-media confirmation。确认失败
时，captured payload 在进入 encoder 或 frame artifact 前就会被丢弃。

## 所有权

边界必须清楚：

| Owner | Owns |
| --- | --- |
| Screenplay | 元素结构、文本、素材引用、cue、局部 delay |
| Rust compiler | parse、bind、reference resolution、精确求时、Timeline IR |
| Runtime | protocol 状态、frame clock、视频解码 readiness、visibility interval |
| Presentation | DOM 形状、CSS、layout、字体与视觉风格 |
| Renderer | materialized asset path、Chromium、capture、encoding |

presentation 收到的 placement 已经包含绝对帧区间。它可以决定 title 长什么样、
CTA 放在哪里、video 怎么被 CSS 布局；它不能把 title 提前、延长 overlay、
重新解释 `delay`，也不能从 DOM 里重新推导媒体时长。

## Authoring facade

`@onmark/authoring` 提供默认语义 DOM bindings：

- `createDomPresentationBindings({ document, videoSource })` 返回 runtime 可消费的
  video/overlay bindings；
- video placement 会变成隐藏的 `<video>`，并带稳定 class `onmark-video`；
- title/CTA placement 会变成隐藏的 `<div>`，并带 `onmark-overlay` 以及
  `onmark-title` 或 `onmark-call-to-action`；
- runtime 根据已求解 interval 切换 visibility，CSS 拥有 layout。

默认 facade 刻意很小。presentation 可以直接实现 `PresentationBindings` 来支持
Canvas、WebGL 或自定义 DOM，但规则不变：binding 创建浏览器资源，`setVisible`
应用可见性，`dispose` 终止性释放资源。

更精确地说，production adapter 会在 `load` 时各调用一次
`bindVideo(placement, index)` 与 `bindOverlay(placement, index)`。video binding 提供
浏览器 element、已 materialize 的 source、visibility effect 和终止性 cleanup；overlay
binding 提供 visibility 与终止性 cleanup。`index` 是 placement 在冻结 plan 中的稳定位置，
仅用于 DOM identity，不是时间坐标。每次 `seek` 时，runtime 先隐藏 video，再根据权威
output frame 选择已准入的 source frame、呈现 ready video，最后应用已求解 overlay 的
visibility。binding 拥有效果，不拥有 interval arithmetic。

## Plan facts、组件选择与 props

当前语言**没有** `presents`、`definePresentation`，也没有 screenplay 到 presentation 的
props 通道。`onmark render` 通过 `--presentation` 或同目录发现选择一个
`presentation.ts`。该 entry 唯一收到的动态事实，是 `Load(plan)` 传入的 Rust-owned
`BrowserPlan`：帧率、evaluation/output interval、video placement 和 overlay placement。
`presentation.ts` 静态 import 的值是 bundled program code，不是 screenplay props。

这项缺失是有意边界，不是未写下来的约定。未来的 presentation selection 或 props feature
必须一起定义：screenplay spelling、带类型的 schema/default、canonical wire encoding、
source span 与 diagnostic、bundle/cache identity，以及与 temporal capability declaration 的
关系；它还必须具备受控 language evaluation 证据。在这些工作完成前，presentation 不得从
global、URL parameter、可变 side channel 或自造的 `presents` attribute 读取作者意图。

## Temporal capability

`stateless`、`warmup` 和 `sequential` 是架构分类，而不是当前公开 adapter API 或
screenplay annotation。production adapter 是唯一由 Gate 一和 Gate 二 conformance 覆盖其
帧行为的 adapter。因此，自定义 adapter 即使实现了 `PresentationBindings`，也不会自动获得
random seek 或 partition 的保证。

temporal capability 成为公开能力时，声明将由 `@onmark/runtime` 拥有，不能在 authoring 或
TypeScript timing code 再复制一份。其语义、证明义务、调度影响和 conformance test 必须和 API
一起落地；任意字符串或 boolean 不是 capability contract。

## 素材

浏览器只看 unit root 下已 materialize 的素材。Gate 一 video source 使用：

```ts
materializedVideoSource(placement);
```

这个 helper 从 Rust-owned browser plan 里的 frozen asset identity 推导
`./assets/sha256/<digest>`。presentation 不应该拼 native path、读取源码文件或假设
working directory。renderer 会在浏览器看到素材前验证字节。

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

CSS animation 暂缓，直到 runtime 有明确的 temporal capability model。依赖加载时刻的
静态 CSS transition 不是 Gate 一确定性输出合约。

## 失败与清理

预期浏览器失败通过 runtime protocol failure 返回。自定义 adapter 如果能识别操作或
readiness 失败，应抛出 `RuntimeAdapterError`；readiness timeout 应携带有界的
pending resource 名称。

dispose 是终止相位。presentation 可以报告清理失败，但不能让半清理状态重新服务。
浏览器 API 允许时，资源清理应保持幂等。

## 非目标

Gate 一不提供 presentation dev server、watch mode、plugin API、component registry、
由 screenplay 选择的组件或 props、跨场景 persist、自由 `begin/end/until` 时间表达式或
browser-side render planning。这些能力必须先有明确语言语义、runtime 合约和评测证据，
才能成为公开契约。
