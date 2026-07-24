# Onmark 语言规格书

> 状态：当前剧本语言。Gate 四准入了现有 authored audio 与 subtitle
> 表面；之后已经完成的 gate 只改变 presentation 与 execution 行为，没有新增剧本拼写。延后语言能力会被明确列出，不与当前语义混写。

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
3. **对齐使用事件。**
   “第 3 秒出现”先成为命名的影片绝对 cue，再由 overlay 引用，不恢复 track
   index。
4. **局部关系保持局部。** 相对当前 shot 的延迟无需跨文档引用。
5. **非法状态尽量不可表达。** 能由结构保证的顺序与归属，不降级成 lint。
6. **剩余错误必须可诊断。**
   每条错误定位源码、解释原因，并尽可能给出源语言级修法。
7. **表面语言不泄漏执行结构。** Render
   Unit、缓存键、轨道和 worker 不属于剧本语法。

## 3. 文档模型

最小词汇：

| 元素    | 含义                               | 时间角色          |
| ------- | ---------------------------------- | ----------------- |
| `film`  | 一部影片及其全局配置               | 根时间域          |
| `cues`  | film 直属的 cue 声明容器，至多一个 | 不参与 scene 顺序 |
| `cue`   | 有名字的时间事件                   | 显式对齐来源      |
| `scene` | 叙事段落                           | 顺序容器          |
| `shot`  | 一次连续画面表达                   | 基本顺序单元      |
| `video` | 当前唯一的视觉媒体元素             | 主内容时长来源    |
| `vo`    | 旁白铭文及对应音频                 | 内容时长来源      |
| `music` | 影片级音乐                         | film 绝对音频     |
| `sfx`   | shot 局部音效                      | shot 局部音频     |
| `title` | 屏幕标题                           | shot 内 overlay   |
| `cta`   | 行动号召                           | shot 内 overlay   |

示意语法：

```html
<om-film>
  <om-music src="score.wav" gain="25%"></om-music>
  <om-cues>
    <om-cue id="offer" time="3s"></om-cue>
    <om-cue id="cta" time="7s"></om-cue>
  </om-cues>

  <om-scene id="sale">
    <om-shot id="hero">
      <video src="product.mp4"></video>
      <om-sfx src="reveal.wav" delay="250ms"></om-sfx>
      <om-title cue="offer">30% OFF</om-title>
      <om-cta cue="cta">立即购买</om-cta>
    </om-shot>
  </om-scene>
</om-film>
```

`cue="offer"` 是 Gate 一中将 overlay 对齐到命名 cue 的正式拼写。自由
`begin`、`end` 和 `until` 表达式不进入语言。

### HTML 语法

剧本是一份作者直接编写的 HTML 文档。普通 HTML 负责 layout 与 presentation，封闭的 Onmark
custom-element 词汇负责剧本语义。作者元素统一使用 `om-` namespace；产品、package 与
artifact 名称仍保留完整 `onmark` 拼写。compiler 直接 tokenise HTML，同时保留源码顺序与精确到
UTF-8 byte 的 span。semantic ownership 不采用 browser 的容错建树；Onmark 自己维护严格的
authored-element stack，避免 malformed presentation markup 静默改变 semantic
ownership。所有非 void authored element 都必须有匹配的结束标签，即使 browser HTML
允许省略该标签。

HTML 元素名和属性名不区分 ASCII 大小写，进入 syntax tree 时使用规范化的小写拼写。comment
被忽略；普通文本、属性、`<style>` 与 `<script>` raw text 都保留 authored span；标准 HTML
character reference 只解码一次。标准 `<!doctype html>` 合法，其他 document type 被拒绝。
trailing solidus 只允许用于 HTML void element；`<om-shot />` 会产生 malformed syntax，
同时像 browser 一样保持该非 void element 处于打开状态。

bind 阶段要求恰好存在一个 `<om-film>` semantic document root。它既可以直接写成 HTML
fragment，也可以作为标准 `html`/`body` document shell 的直接子元素；只有这层 shell 是透明
的，把 film 藏进 `div` 等 presentation container 不会改变 screenplay ownership。普通 HTML
sibling、document text、`head` 与 presentation descendant 始终由 presentation 拥有，不进入
linked film。root cardinality、已知 `om-*` 名称、合法包含关系、必需属性、ID 与引用属于
语言语义。`<om-title>`、`<om-cta>` 与 `<om-vo>` 内的 native descendant 只按源码
顺序贡献文本。

标记输入在语义 bind 之前就有明确上界：单份剧本最多包含 8 MiB UTF-8
源码、65,536 个被保留的 syntax item，以及 32 个同时打开的元素。越过任一上界只产生一条稳定的资源诊断并停止 syntax recovery；编译器不会保留或递归进入被拒绝的后缀。

## 4. 包含与顺序

### Film

`film` 建立根时间域和 cue 作用域。帧率由编译选项决定，画幅由 `RenderProfile`
决定；二者都不是 `film` 属性。它可以有至多一个直属 `cues` 子元素；`cues` 只包含
`cue`
声明，不参与 scene 顺序。film 还可直接拥有不参与 scene 顺序的 `music`。顶层 scene
按源码顺序播放。可渲染的 film 必须至少包含一个求解后持续时间为正的 shot。

### Scene

scene 表达叙事分组，不等于渲染分片。scene 内的 shot 默认顺序播放。scene 时长由其 shot 推导。

### Shot

shot 是最小的默认顺序单元。它拥有一个局部时间原点，并拥有 `video`、`vo`、`sfx`、`title`
和 `cta` 内容；其子内容的 `delay` 都相对这个原点解释。当前只有 `video`
这一种视觉媒体元素；通用音频由语义明确的 `music` 与 `sfx` 表达，泛化的 `audio`
元素不进入词汇。image 等其他媒体元素仍延后。结构 bind 会保留 `src`
及其他尚未解析的 authored
attributes，交给后续属性/引用 resolve 相位，而不会在相位边界丢失。shot 可以成为缓存候选边界，但语言不承诺它是独立 Render
Unit。

### Overlay

title、cta 等 overlay 属于 shot，但不参与兄弟 shot 的顺序排列。overlay 从已解析的
`cue`、`delay`
起点开始；没有显式关系时从所属 shot 起点开始，并持续到该 shot 的 exclusive
end。Gate 一不给 overlay 设置独立的默认时长。因此 overlay 不能延长所属 shot，解析后的起点落在 shot 外时必须报告 authored
timing error。

## 5. 时间来源

作者时间值采用精确文法
`整数[.小数](s|ms)`，不允许空白或正负号。秒最多九位小数，毫秒最多六位小数，因此每个合法值都能精确表示为无符号整数纳秒。shot 的
`duration` 必须大于零；cue time 与 delay 可以为零。语言不接受帧单位或浮点近似。

编译器使用整数运算把精确纳秒映射到有理数帧网格。每次换算都必须在调用点明确选择向下或向上取整；隐式 cast 或环境默认值不得决定帧边界。Gate 一的 authored 起点、delay、cue
time 和 duration 都选择不早于精确值的第一个帧边界（`Ceil`），因此正的亚帧值不会被静默压成零帧。`Floor`
只保留给明确要求归属到更早边界的规则。

每个 shot 的持续时间必须来自一种可溯源规则。Gate 一允许：

1. **媒体驱动**：视频或音频素材的探测时长；
2. **旁白驱动**：`vo` 引用音频的探测时长；
3. **显式定长**：没有内容时长来源时，使用受限的 `duration`。

Gate 一不允许 shot 结束于 cue，也不实现由总长或 flex 约束进行的容器求解。

同一 shot 存在多个内容时长来源时，默认持续到最长主内容结束。哪些元素是“主内容”、哪些只是 overlay，必须由元素类型确定，不能靠 DOM 位置猜测。

所有推导都进入 `TimingReason`：

```text
shot hero: 0f..210f
because video product.mp4 = 210f

title offer: 90f..210f
because cue offer = 90f
and owning shot ends = 210f
```

## 6. 两种显式时间关系

### 局部延迟

`delay` 表示相对所属 shot 起点的偏移：

```html
<om-shot>
  <om-title delay="3s">30% OFF</om-title>
</om-shot>
```

它不改变 shot 的起点，也默认不延长 shot。若结果越过 shot 末尾，编译器报错并指出需要延长哪个内容或改用哪个 cue。

### 命名 cue

cue 把业务时间命名后供 overlay 对齐。Gate 一的 cue 只来自作者声明的影片绝对时间，不存在其他当前来源。

普通元素不直接书写裸绝对秒数；“第 3 秒出现 30% OFF”先成为 `offer = 3s`，元素引用
`offer`。这样数字集中、可命名、可复用，也更容易诊断。

## 7. 旁白是意图与产物的配对

```html
<om-vo src="vo_01_a3f2.mp3">三年前，我们只有三个人。</om-vo>
```

- 文本是铭文：用于阅读、审阅、字幕派生和修改；
- `src` 是 screenplay-relative portable path：只使用 `/`
  分隔，不能是绝对路径，不能含
  `..`、空组件、`.`、反斜杠或平台前缀；它指向冻结媒体产物，用于实际渲染和时长探测，且必须含有音轨；否则 solve 会在
  `src` 位置报告 `ONM-ASSET-002`；
- 编译器不调用 TTS，不访问网络；
- 上游 agent 负责在文字变化后重新生成素材；
- 内容 hash 或 manifest 用于发现文字与素材版本不一致。

Gate 一会把每条已经求解的旁白素材复制到私有 render
root，并在浏览器捕获之后按其求解出的帧区间混入输出。presentation 不播放、不延迟、也不混音旁白。

没有 `src`
时，是否允许只做静态检查而禁止渲染，需要由命令模式决定，不能静默假设一个朗读速度。

### 通用音频

`music` 与 `sfx` 是两种不同的作者语义，不是带自由 `kind`
属性的泛化元素。元素类型直接保证 role 与 parent 的合法组合，同时保持 narrative `vo`
是独立概念。

film 可以拥有任意数量的直属 `music`。music 从影片零帧开始，以引用音频流的探测时长为自然持续时间，并可跨越 scene、shot 与 Render
Unit。music 不延长影片：素材长于已求解影片时在影片 exclusive end 截止，素材较短时自然结束；music 不接受 authored delay。

shot 可以拥有任意数量的直属 `sfx`。音效从 shot 起点加可选局部 `delay`
开始，以素材探测时长决定 exclusive end；它既不决定也不延长 shot 时长。音效起点或末端越过所属 shot 时必须报告 authored timing
error，不能静默裁掉尾部。

两种元素都必须提供 screenplay-relative `src`，路径规则与 voice-over 相同。可选 `gain`
采用精确文法 `整数%`，范围为 `0%` 到 `100%`（含端点），默认 `100%`；它表示线性振幅比例，不是分贝。引用素材必须含有音轨。混音和 mux
属于原生执行边界，浏览器不播放这些元素。

## 8. ID、作用域与引用

- 所有显式 ID（包括 cue ID）在 film 内共享一个全局唯一的声明空间；
- ID 不得为空，且遵循 HTML `id` 的基础约束，不得包含 ASCII whitespace；
- ID 区分大小写；非 ASCII 字符按原文保留，编译器不得静默规范化；
- cue 引用通过 `CueId`/`EventRef`
  与普通节点引用作类型区分，但不建立可重名的第二声明空间；
- 删除被引用 cue 是编译错误；
- 未使用 cue 默认 warning；
- 编译后字符串变成稳定 `NodeId`/`CueId`，核心层不继续传字符串。

### 属性与引用解析

结构 bind 之后执行属性与引用 resolve。`film`、`cues`、`scene`
不接受 ID 以外的属性；`cue` 必须有 `id` 与 `time`；`shot` 可有
`duration`；`video`、`vo` 可有 `src` 与 `delay`；`title`、`cta` 可有 `cue` 或
`delay`；`music` 必须有 `src`，可有 `gain`；`sfx` 必须有 `src`，可有 `delay`
与 `gain`。同一个 overlay 不能同时写 `cue` 与
`delay`，因为两者定义互相竞争的起点规则。`video` 或 `vo` 缺少 `src`
时仍可用于静态分析；`music` 与 `sfx` 则在 resolve 阶段要求 `src`。任何元素显式写空
`src` 都非法。未知属性一律报错。

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

字段通过只读 accessor 暴露，构造器拒绝全空白的 message、help 和 related
message。severity 由稳定 diagnostic
code 决定，调用者不能把同一个 code 随意标成 error 或 warning。

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

| Code             | 含义                                                            |
| ---------------- | --------------------------------------------------------------- |
| `ONM-SYNTAX-001` | 标记格式错误，无法继续产出可信 token                            |
| `ONM-SYNTAX-002` | 结束标签与当前打开元素不匹配                                    |
| `ONM-SYNTAX-003` | 同一元素重复声明属性名                                          |
| `ONM-SYNTAX-004` | 文本或属性中存在非法字符引用或实体引用                          |
| `ONM-SYNTAX-005` | 输入结束时仍有元素未闭合                                        |
| `ONM-SYNTAX-006` | 结束标签没有对应的打开元素                                      |
| `ONM-SYNTAX-007` | 不支持非 HTML document type |
| `ONM-SYNTAX-008` | 剧本标记越过有界 syntax 资源                                    |

首批 bind、resolve 与 timing 诊断为：

| Code             | 含义                                          |
| ---------------- | --------------------------------------------- |
| `ONM-ID-001`     | ID 为空或包含 ASCII whitespace                |
| `ONM-ID-002`     | ID 与同一 film 内的另一条声明重复             |
| `ONM-STRUCT-001` | 元素不属于当前剧本词汇                        |
| `ONM-STRUCT-002` | 文档没有 semantic `film` root                 |
| `ONM-STRUCT-003` | 文档包含多个 semantic `film` root             |
| `ONM-STRUCT-004` | 已知元素不在合法父元素内                      |
| `ONM-STRUCT-005` | film 包含多个 `cues` 容器                     |
| `ONM-STRUCT-006` | 结构元素或空元素中出现了文本                  |
| `ONM-TIME-001`   | 时长格式非法、精度过高或超出精确范围          |
| `ONM-TIME-002`   | shot 没有媒体推导或显式的时长来源             |
| `ONM-TIME-003`   | 显式 shot duration 与媒体推导时长互相竞争     |
| `ONM-TIME-004`   | 已解析 shot 内容的起点或末端落在所属 shot 外  |
| `ONM-TIME-005`   | 精确时间无法装入所选帧域                      |
| `ONM-TIME-006`   | film 没有求解出任何持续时间为正的 shot        |
| `ONM-ASSET-001`  | 可渲染媒体没有冻结素材引用                    |
| `ONM-ASSET-002`  | 媒体元素引用的素材没有所需轨道                |
| `ONM-REF-001`    | 格式良好的 overlay cue 引用没有指向已解析 cue |
| `ONM-REF-002`    | 已解析 cue 从未被引用                         |
| `ONM-ATTR-001`   | 元素包含未知属性                              |
| `ONM-ATTR-002`   | 元素缺少必需属性                              |
| `ONM-ATTR-003`   | 属性值非法，包括格式错误的 cue ID             |
| `ONM-ATTR-004`   | 两个属性定义了互相冲突的规则                  |
| `ONM-CAPTION-001` | 导入的字幕文件违反所选格式的语法             |
| `ONM-CAPTION-002` | 导入的字幕文件使用了尚未支持的呈现语义       |
| `ONM-CAPTION-003` | 导入的字幕文件超过了有界摄取限制             |

`ONM-REF-002`
的 severity 是 warning；其余首批 bind、resolve 与 timing 诊断均为 error。

tokenizer 遇到致命词法错误后停止，因此词法恢复可能只能产生一条诊断；只要剩余结构可信，Onmark 仍继续聚合彼此独立的嵌套、绑定和语义诊断。输入结束时，每个仍处于打开状态的元素各产生一条诊断：primary
span 指向元素的打开名称，related span 指向剧本末尾。即使 tokenizer 将 document
type 的内部子集拆成多个 token，整段声明也只产生一条诊断。

编译器在安全时聚合独立错误。禁止为了更快返回而让 LLM 经历“一次只修一个 typo”的循环。

## 10. Presentation 与 props

作者的 HTML 同时就是 presentation。Onmark 把已求解的 video、title、CTA 与 caption fact
绑定到现有 semantic element，不替换普通 DOM、class、嵌套 markup 或 inline style。可选的
`<script type="module" data-om-motion>` 只导出一个 `motion` value，并可导入
`onmark/motion/gsap` 等已准入 adapter；bundler 不允许其他 script element。

当前没有同名 CSS/motion 文件约定、`--presentation` escape hatch、`presents` attribute、
`definePresentation` declaration 或独立 typed props channel。已求解事实只作为
Rust-owned `BrowserPlan`，通过 runtime 的 `Load(plan)` 到达浏览器。

Browser Plan 还会保留 film、scene、shot 与 content ownership。compiler 为每个投影 node
分配稳定 identity，并且只携带已准入的 authored ID、语义角色、text、ownership 与 solved
interval。这既不是通用 screenplay props channel，也不是第二条 presentation timeline。

这是语言边界，不是未写下来的实现细节。未来的 screenplay-selected
presentation 或 props feature 必须一起定义其 spelling、typed
schema/default、canonical encoding、带 source 的 diagnostic、bundle/cache
identity 和 temporal capability
effect，并满足下方语言实验门槛。在那之前，stylesheet rule 与静态 TypeScript import
都是 presentation code，不是 screenplay props。浏览器 authoring contract 另见
[presentation contract](presentation-contract.md)。

## 11. 不进入 Gate 一的能力

- 自由 `begin`、`end` 和 `until` 时间表达式；
- shot 结束于 cue；
- 从素材分析、上游事件表或类型化语义边界生成 cue；
- 任意负 offset；
- flex 与一般线性方程混用；
- 条件分支和运行时未知数量循环；
- speed ramp、倒放和音频响应式动画；
- 跨 scene persist 和内容感知转场；
- screenplay 选择的 presentation 或 props；
- 自动 TTS 或联网素材生成。

这些不是永远拒绝，而是必须通过真实用例、语义设计和生成实验进入语言。未来的类型化语义边界仍然只能产生命名事件，不会恢复自由时间属性；不能用任意属性逃逸口提前吞掉它们。

## 12. 语言实验门槛

任何新增语法先回答：

1. 它表达的是新领域概念，还是旧概念的补丁？
2. 能否归约到现有包含、顺序、内容时长或事件关系？
3. 是否制造互相矛盾的新组合？
4. 人从上到下朗读时是否仍理解影片？
5. LLM 在相同题目和相同说明下，是否比现有写法更可靠？
6. 编译器能否给出局部、可执行的修复建议？

表面拼写通过固定题、真实广告版式题、编辑变体和 OOV 题评测。不能仅凭纸面优雅进入稳定语言。

语言评测是仓库数据，不是口头结论。语法提案改变 Gate 一表面语言之前，必须提交可复现的题目、prompt、grader、原始输出、模型参数和对照 baseline。CI 可以校验并重新计分这些冻结资产，但不必调用在线模型。

## 13. 与渲染架构的边界

语言编译到 Timeline
IR 后结束职责。它不选择 Chromium、不决定 worker 数、不暴露轨道、不设置 GOP，也不承诺 shot 独立缓存。

渲染架构可以改变 Render Graph、分片和编码策略，只要相同 Timeline
IR 的可见语义不变。语言可以演进表面词汇，只要 versioned IR
migration 明确。这个边界允许语言与渲染管线各自优化，而不互相泄漏实现细节。
