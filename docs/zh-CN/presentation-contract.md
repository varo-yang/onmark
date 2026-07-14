# Onmark Presentation Contract

> 状态：Gate 一浏览器 authoring 合约。

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

## Runtime 握手

presentation 必须用 `installRuntimeHost` 安装一个 runtime host。native renderer
等待该 host 后，发送版本化 browser protocol：

```text
Load(plan) -> Prepare(evaluationStart) -> Seek(frame)* -> Dispose
```

`FrameReady(frame)` 表示 runtime 已为这个精确帧应用选中的状态。它不表示
presentation 自己算出了时间。capture 前，native executor 会进行一次有界的两个
animation-frame compositor commit wait；该等待不选择时间，只确保 Chromium 已将选中的
状态提交到 capture surface。

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
跨场景 persist、自由 `begin/end/until` 时间表达式或 browser-side render planning。
这些能力必须先有明确语言语义、runtime 合约和评测证据，才能成为公开契约。
