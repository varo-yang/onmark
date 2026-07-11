# Onmark 语言规格书

> 状态：语义草案。本文先固定语言模型、时间语义和诊断契约；尚未经过生成实验验证的表面拼写会明确标注为 provisional。

## 1. 语言为什么存在

Onmark 语言的目标不是用另一套标签重画时间轴，而是让创作者表达影片本身，让编译器承担绝对坐标、约束维护和错误解释。

```text
创作者负责：内容、顺序、归属、少量必须明确的对齐关系
编译器负责：素材探测、时长推导、绝对帧、冲突检测、Render IR
```

语言服务两类作者：人和 LLM。两者都应当能从上到下朗读文档、局部修改内容，并从 diagnostics 直接找到修法。

## 2. 语言公理

1. **顺序是默认。** 相邻 shot 首尾衔接，不重复声明绝对起点。
2. **内容决定时长。** 旁白和媒体已有时长时，编译器探测并使用它。
3. **对齐使用事件。** “第 3 秒出现”和“旁白结束后出现”都是对命名时间事件的关系，不恢复 track index。
4. **局部关系保持局部。** 相对当前 shot 的延迟无需跨文档引用。
5. **非法状态尽量不可表达。** 能由结构保证的顺序与归属，不降级成 lint。
6. **剩余错误必须可诊断。** 每条错误定位源码、解释原因，并尽可能给出源语言级修法。
7. **表面语言不泄漏执行结构。** Render Unit、缓存键、轨道和 worker 不属于剧本语法。

## 3. 文档模型

最小词汇：

| 元素 | 含义 | 时间角色 |
| --- | --- | --- |
| `film` | 一部影片及其全局配置 | 根时间域 |
| `cues` | film 直属的 cue 声明容器，至多一个 | 不参与 scene 顺序 |
| `cue` | 有名字的时间事件 | 显式对齐来源 |
| `scene` | 叙事段落 | 顺序容器 |
| `shot` | 一次连续画面表达 | 基本顺序单元 |
| `video` | Gate 一唯一的媒体元素 | 主内容时长来源 |
| `vo` | 旁白铭文及对应音频 | 内容时长来源 |
| `title` | 屏幕标题 | shot 内 overlay |
| `cta` | 行动号召 | shot 内 overlay |

示意语法：

```html
<film>
  <cues>
    <cue id="offer" time="3s" />
    <cue id="cta" time="7s" />
  </cues>

  <scene id="sale">
    <shot id="hero">
      <video src="product.mp4" />
      <title cue="offer">30% OFF</title>
      <cta cue="cta">立即购买</cta>
    </shot>
  </scene>
</film>
```

这里固定的是“元素可以对齐到命名 cue”的语义。`cue="offer"` 的最终属性拼写仍是 provisional，必须经过 LLM 对照实验后定案；自由 `begin/end` 表达式不进入默认语言。

### 标记语法

剧本使用 XML-compatible fragment markup。syntax 保留顶层节点序列，并校验 token、元素嵌套、结束标签匹配、重复属性和字符引用。bind 阶段再要求顶层恰好存在一个 `film`；根节点数量、已知元素名、合法包含关系、必需属性、ID 和引用都属于语言语义，不属于标记良构性。

元素名和属性名是区分大小写的 qualified name。语法树拥有解码后的文本和属性值，并保留精确到 UTF-8 byte 的源码 span。comment 被忽略，CDATA 变成普通文本；XML declaration、processing instruction 和 document type declaration 不进入剧本表面语言。

文本和属性值支持 XML 的五个预定义实体（`amp`、`lt`、`gt`、`quot`、`apos`），以及指向 XML 1.0 合法字符的十进制、十六进制引用。其他命名实体、格式错误的引用、surrogate、越过 Unicode 范围或被 XML 禁止的字符都是语法错误。Onmark 不处理 DTD、自定义实体和外部实体。

## 4. 包含与顺序

### Film

`film` 建立根时间域、帧率、画幅和 cue 作用域。它可以有至多一个直属 `cues` 子元素；`cues` 只包含 `cue` 声明，不参与 scene 顺序。顶层 scene 按源码顺序播放。

### Scene

scene 表达叙事分组，不等于渲染分片。scene 内的 shot 默认顺序播放。scene 时长由其 shot 推导。

### Shot

shot 是最小的默认顺序单元。它拥有一个局部时间原点，并拥有 `video`、`vo`、`title` 和 `cta` 内容；其子内容的 `delay` 都相对这个原点解释。Gate 一只有 `video` 这一种媒体元素，使用 `src` 指向素材；audio、image 等媒体元素暂不进入词汇。结构 bind 会保留 `src` 及其他尚未解析的 authored attributes，交给后续属性/引用绑定切片，而不会在相位边界丢失。shot 可以成为缓存候选边界，但语言不承诺它是独立 Render Unit。

### Overlay

title、cta 等 overlay 属于 shot，但不参与兄弟 shot 的顺序排列。默认被所属 shot 裁剪，不能通过自身延迟静默延长整部影片。

## 5. 时间来源

作者时长采用精确文法 `整数[.小数](s|ms)`，不允许空白或正负号。秒最多九位小数，毫秒最多六位小数，因此每个合法值都能精确表示为无符号整数纳秒；语言不接受帧单位或浮点近似。

每个 shot 的持续时间必须来自一种可溯源规则。v0 允许：

1. **媒体驱动**：视频或音频素材的探测时长；
2. **旁白驱动**：`vo` 引用音频的探测时长；
3. **显式定长**：没有内容时长来源时，使用受限的 `duration`；
4. **结束事件**：shot 延续到一个已定义 cue；
5. **容器求解**：未来由总长/flex 约束求得，v0 不实现。

同一 shot 存在多个内容时长来源时，默认持续到最长主内容结束。哪些元素是“主内容”、哪些只是 overlay，必须由元素类型确定，不能靠 DOM 位置猜测。

所有推导都进入 `TimingReason`：

```text
shot hero: 0f..210f
because video product.mp4 = 210f

title offer: 90f..150f
because cue offer = 90f
and default title duration = 60f
```

## 6. 两种显式时间关系

### 局部延迟

`delay` 表示相对所属 shot 起点的偏移：

```html
<shot>
  <title delay="3s">30% OFF</title>
</shot>
```

它不改变 shot 的起点，也默认不延长 shot。若结果越过 shot 末尾，编译器报错并指出需要延长哪个内容或改用哪个 cue。

### 命名 cue

cue 把业务时间命名后供元素对齐。cue 的来源可以是：

- 影片绝对时间；
- 素材分析产生的 beat/marker；
- 某个语义节点的 begin/end；
- 上游工具提供并冻结的事件表。

v0 只实现影片绝对时间 cue，其他来源进入后续规格，但共享同一个内部 `EventRef` 模型。

普通元素不直接书写裸绝对秒数；“第 3 秒出现 30% OFF”先成为 `offer = 3s`，元素引用 `offer`。这样数字集中、可命名、可复用，也更容易诊断。

## 7. 旁白是意图与产物的配对

```html
<vo src="vo_01_a3f2.mp3">三年前，我们只有三个人。</vo>
```

- 文本是铭文：用于阅读、审阅、字幕派生和修改；
- `src` 是冻结媒体产物：用于实际渲染和时长探测；
- 编译器不调用 TTS，不访问网络；
- 上游 agent 负责在文字变化后重新生成素材；
- 内容 hash 或 manifest 用于发现文字与素材版本不一致。

没有 `src` 时，是否允许只做静态检查而禁止渲染，需要由命令模式决定，不能静默假设一个朗读速度。

## 8. ID、作用域与引用

- 所有显式 ID（包括 cue ID）在 film 内共享一个全局唯一的声明空间；
- ID 不得为空，且遵循 HTML `id` 的基础约束，不得包含 ASCII whitespace；
- ID 区分大小写；非 ASCII 字符按原文保留，编译器不得静默规范化；
- cue 引用通过 `CueId`/`EventRef` 与普通节点引用作类型区分，但不建立可重名的第二声明空间；
- 删除被引用 cue 是编译错误；
- 未使用 cue 默认 warning；
- 编译后字符串变成稳定 `NodeId`/`CueId`，核心层不继续传字符串。

### 属性与引用解析

结构 bind 之后执行属性与引用 resolve。`film`、`cues`、`scene` 不接受 ID 以外的属性；`cue` 必须有 `id` 与 `time`；`shot` 可有 `duration`；`video`、`vo` 可有 `src` 与 `delay`；`title`、`cta` 可有 `cue` 或 `delay`。同一个 overlay 不能同时写 `cue` 与 `delay`，因为两者定义互相竞争的起点规则。缺少媒体 `src` 仍可用于静态分析，但显式空 `src` 非法。未知属性一律报错。

## 9. 诊断契约

```rust
pub struct Diagnostic {
    code: DiagnosticCode,
    primary: SourceSpan,
    message: Box<str>,
    help: Option<Box<str>>,
    related: Vec<RelatedDiagnostic>,
}
```

字段通过只读 accessor 暴露，构造器拒绝全空白的 message、help 和 related message。severity 由稳定 diagnostic code 决定，调用者不能把同一个 code 随意标成 error 或 warning。

诊断必须使用作者看得见的词汇。反例：

```text
constraint graph node 17 is unsatisfied
```

正例：

```text
ONM-TIME-004 标题“立即购买”从第 13 秒开始，但所属 shot 在第 12 秒结束。
建议：延长 shot “closing”，或把该标题对齐到更早的 cue。
```

主要诊断类别：语法错误、未知元素/属性、重复 ID、未知 cue、时间越界、无时长来源、时长冲突、素材缺失、铭文与产物不一致、暂不支持的表达。

首批标记诊断为：

| Code | 含义 |
| --- | --- |
| `ONM-SYNTAX-001` | 标记格式错误，无法继续产出可信 token |
| `ONM-SYNTAX-002` | 结束标签与当前打开元素不匹配 |
| `ONM-SYNTAX-003` | 同一元素重复声明属性名 |
| `ONM-SYNTAX-004` | 文本或属性中存在非法字符引用或实体引用 |
| `ONM-SYNTAX-005` | 输入结束时仍有元素未闭合 |
| `ONM-SYNTAX-006` | 结束标签没有对应的打开元素 |
| `ONM-SYNTAX-007` | 不支持 XML declaration、processing instruction 或 document type |

首批 bind 与 resolve 诊断为：

| Code | 含义 |
| --- | --- |
| `ONM-ID-001` | ID 为空或包含 ASCII whitespace |
| `ONM-ID-002` | ID 与同一 film 内的另一条声明重复 |
| `ONM-STRUCT-001` | 元素不属于 Gate 一词汇 |
| `ONM-STRUCT-002` | 文档没有顶层 `film` 元素 |
| `ONM-STRUCT-003` | 文档包含多个顶层 `film` 元素 |
| `ONM-STRUCT-004` | 已知元素不在合法父元素内 |
| `ONM-STRUCT-005` | film 包含多个 `cues` 容器 |
| `ONM-STRUCT-006` | 结构元素或空元素中出现了文本 |
| `ONM-TIME-001` | 时长格式非法、精度过高或超出精确范围 |
| `ONM-REF-001` | overlay 的 cue 引用没有指向已解析 cue |
| `ONM-REF-002` | 已解析 cue 从未被引用 |
| `ONM-ATTR-001` | 元素包含未知属性 |
| `ONM-ATTR-002` | 元素缺少必需属性 |
| `ONM-ATTR-003` | 属性值非法 |
| `ONM-ATTR-004` | 两个属性定义了互相冲突的规则 |

`ONM-REF-002` 的 severity 是 warning；其余首批 bind 与 resolve 诊断均为 error。

tokenizer 遇到致命词法错误后停止，因此词法恢复可能只能产生一条诊断；只要剩余结构可信，Onmark 仍继续聚合彼此独立的嵌套、绑定和语义诊断。输入结束时，每个仍处于打开状态的元素各产生一条诊断：primary span 指向元素的打开名称，related span 指向剧本末尾。即使 tokenizer 将 document type 的内部子集拆成多个 token，整段声明也只产生一条诊断。

编译器在安全时聚合独立错误。禁止为了更快返回而让 LLM 经历“一次只修一个 typo”的循环。

## 10. 不进入 v0 的能力

- 自由 `begin/end` 时间表达式；
- 任意负 offset；
- flex 与一般线性方程混用；
- 条件分支和运行时未知数量循环；
- speed ramp、倒放和音频响应式动画；
- 跨 scene persist 和内容感知转场；
- 自动 TTS 或联网素材生成。

这些不是永远拒绝，而是必须通过真实用例、语义设计和生成实验进入语言。不能用任意属性逃逸口提前吞掉它们。

## 11. 语言实验门槛

任何新增语法先回答：

1. 它表达的是新领域概念，还是旧概念的补丁？
2. 能否归约到现有包含、顺序、内容时长或事件关系？
3. 是否制造互相矛盾的新组合？
4. 人从上到下朗读时是否仍理解影片？
5. LLM 在相同题目和相同说明下，是否比现有写法更可靠？
6. 编译器能否给出局部、可执行的修复建议？

表面拼写通过固定题、真实广告版式题、编辑变体和 OOV 题评测。不能仅凭纸面优雅进入稳定语言。

## 12. 与渲染架构的边界

语言编译到 Timeline IR 后结束职责。它不选择 Chromium、不决定 worker 数、不暴露轨道、不设置 GOP，也不承诺 shot 独立缓存。

渲染架构可以改变 Render Graph、分片和编码策略，只要相同 Timeline IR 的可见语义不变。语言可以演进表面词汇，只要 versioned IR migration 明确。这个边界允许语言与渲染管线各自优化，而不互相泄漏实现细节。
