# Onmark Rust 代码宪法

> 基线：Rust 1.97.0 stable（2026-07-09）、语言 edition 2024、style edition
> 2024。英文版是规范原文；每次更新锁定工具链时，本规范必须重新审计。

优美的 Rust 不是字符最少的 Rust，而是所有权、失败方式和状态转换都能从类型与控制流中直接读出的 Rust。Onmark 是确定性视频编译器和渲染器；本规范优先保障可修改性、跨机器可复现性和有界资源执行。

## 这份规范的三层含义

不要把三种“Rust 风格”混为一谈：

1. **官方排版**交给 `rustfmt` 的 style edition 2024，人不发明对齐规则。
2. **地道设计**管理调用点清晰度、所有权、typestate、错误、trait 和资源生命周期。
3. **Onmark 工程法**额外要求确定时间、规范计划、有界媒体 buffer 和子进程纪律。

第一层是机械排版，第二层是代码美学，第三层是产品正确性。通过 `cargo fmt`
的代码仍可能同时违反后两层。

## 现代 Rust 代码美学

### 调用点是 API 的第一设计界面

读调用的次数远多于读声明。优先让意义在调用点直接可见：

```rust
timeline.resolve(cues, Rounding::TowardZero)?;
worker.capture(frame, CaptureMode::BeginFrame)?;
```

不要为了让声明少五行，让所有调用者理解炫技的泛型推导。标准库已有词汇优先于项目新造同义词。

### Parse，而不是反复 validate

外部文本只转换一次，得到能证明不变式的类型。`CueId::parse` 返回的必须是有效
`CueId`；下游不再接受 `String` 然后重复检查。

### Typestate 只表达有意义的协议状态

```rust
CaptureSession<Launched> -> CaptureSession<Ready> -> CaptureSession<Closed>
```

当独立状态类型能消灭真实运行时分支或防止协议误用时才使用。不要把每个小实现步骤都 typestate 化；若各状态操作相同且无法误用，enum 更清晰。

### 所有权尽量形成树

主动分离 owned/view 形式（`PathBuf`/`Path`、拥有的 IR/IR 视图）。长生命所有权必须可见，借用尽量短；循环图优先使用 index 或稳定 ID，不建自引用结构。`Cow`
只在借用和拥有两条路都真实存在时使用。

### RAII 拥有清理

文件、临时目录、浏览器 session、encoder、permit 和 trace span 都是资源。默认通过
`Drop` 或显式 async shutdown guard 清理。但必须被调用者观测的失败，不能藏在
`Drop` 里。

### 根据集合是否开放选择多态

- Onmark 已知的封闭集合：enum + 穷尽 `match`。
- 编译期开放行为：generic parameter。
- 运行时选择的开放行为：窄 trait object。
- 为外部类型添加行为：extension trait。

不要把 class hierarchy 机械翻译成 trait hierarchy。

### 有意识地使用 Rust 2024

- Rust 2024 的 return-position `impl Trait`
  默认捕获作用域中所有泛型和生命期。公开 API 需承诺“不保留某借用”时使用精确
  `use<...>` capture。
- `unsafe fn` 内部的 unsafe 操作仍必须写显式 `unsafe {}`。
- 禁止引用 `static mut`；按语义使用从 `main`
  传入的所有权、atomic、`Mutex`、`OnceLock` 或 `LazyLock`。
- 优先穷尽 match。Rust 2024 match
  ergonomics 不是在密集 pattern 中隐藏所有权变化的理由。
- async
  closure 等现代语法只在它真正简化调用点所有权时使用；“新”本身不是使用理由。

## 代码美学对照手册

这些不是语法展示，而是 code
review 的判断规则。“推荐”意味着调用点直接暴露领域意图，并让错误状态更难被构造。

### 代码应该拥有方正的轮廓

Onmark 偏好**矩形代码**：正常路径笔直、缩进浅、代码块完整，同层只有少量清晰并列的阶段。这不只是“多用早返回”。读者应该在逐句阅读之前，就能先看懂函数的形状。

不推荐不断收窄的金字塔：

```rust
for scene in film.scenes() {
    if scene.enabled() {
        if let Some(asset) = assets.get(scene.asset_id()) {
            match probe(asset).await {
                Ok(metadata) => {
                    if metadata.duration > Duration::ZERO {
                        plans.push(build_plan(scene, metadata)?);
                    } else {
                        diagnostics.push(empty_asset(scene));
                    }
                }
                Err(error) => diagnostics.push(probe_failed(scene, error)),
            }
        } else {
            diagnostics.push(missing_asset(scene));
        }
    }
}
```

推荐由完整、对齐的代码块依次组成：

```rust
for scene in film.enabled_scenes() {
    let Some(asset) = assets.get(scene.asset_id()) else {
        diagnostics.push(missing_asset(scene));
        continue;
    };

    let metadata = match probe(asset).await {
        Ok(metadata) => metadata,
        Err(error) => {
            diagnostics.push(probe_failed(scene, error));
            continue;
        }
    };

    if metadata.duration == Duration::ZERO {
        diagnostics.push(empty_asset(scene));
        continue;
    }

    plans.push(build_plan(scene, metadata)?);
}
```

这条视觉规则有四个具体推论：

1. **函数是矩形。**
   正常路径保持在同一缩进层。拒绝、跳过和错误转换发生在异常出现的边界。
2. **代码块是完整的。**
   识别、校验、转换、产出是肉眼可见的阶段，禁止把四者的碎片交错塞进嵌套 closure。
3. **分支是均衡的。** 一个大 `match`
   分支内部又出现决策树时，把它提炼成有领域名字的操作；没有领域含义的一行 helper 并不会让代码更好。
4. **模块形成树，而不是撒成纸屑。**
   因同一原因变化的代码放在一起。禁止只为缩短函数，就把一次操作拆散到
   `utils`、extension trait、callback 和多个文件。

顶层编排应该像目录一样一眼可读：

```rust
pub async fn render(request: RenderRequest) -> Result<RenderOutput, RenderError> {
    let source = load_source(&request).await?;
    let film = compile_film(source)?;
    let assets = resolve_assets(&film).await?;
    let plan = build_render_plan(film, assets)?;
    let segments = render_segments(&plan).await?;

    assemble_output(segments, &plan).await
}
```

不要为了追逐行数指标机械抽 helper。只有当一个代码块拥有稳定的领域名称、因不同原因变化、能够建立有价值的类型边界，或封装了完整的机械细节时才抽取。我们追求的层次是：**函数呈矩形，模块呈树形，管线呈线性**。

### 用名字表达选择，不用布尔值编码

不推荐：

```rust
capture_frame(frame, true, false, 3)?;
```

推荐：

```rust
capture_frame(
    frame,
    CaptureOptions {
        trigger: CaptureTrigger::BeginFrame,
        alpha: AlphaMode::Opaque,
        retries: RetryLimit::new(3),
    },
)?;
```

第二个调用无需跳进函数定义就能 review。多个独立开关使用 options
struct；互斥选择使用 enum。

### 只在边界解析一次，之后相信类型

不推荐让文本时间穿过整个编译器：

```rust
fn resolve_cue(name: &str, seconds: f64) -> Result<f64, String> {
    if !name.starts_with("cue:") || seconds < 0.0 {
        return Err("invalid cue".into());
    }
    Ok(seconds)
}
```

推荐在语法边界建立领域类型：

```rust
pub struct CueName(String);
pub struct TimelineTime(Duration);

impl TryFrom<&str> for CueName {
    type Error = InvalidCueName;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value
            .strip_prefix("cue:")
            .filter(|name| !name.is_empty())
            .map(|_| Self(value.to_owned()))
            .ok_or(InvalidCueName)
    }
}

fn resolve_cue(name: &CueName, at: TimelineTime) -> ResolvedCue;
```

解析完成后，核心代码不应反复追问 cue 和时间是否合法。

### 让阶段消费阶段

不推荐用一个可变大对象承载全部编译阶段：

```rust
let mut document = Document::parse(source)?;
document.resolve_assets()?;
document.solve_timeline()?;
let plan = document.plan.expect("timeline was solved");
```

推荐用阶段类型：

```rust
let parsed = ParsedFilm::parse(source)?;
let linked = parsed.link_assets(&assets)?;
let solved = linked.solve_timeline()?;
let plan: RenderPlan = solved.lower();
```

消费上一个阶段，能从类型上禁止“未求时就 lower”，也不再需要一堆只表示工作流状态的
`Option`。

### 用 enum 表达备选规则，不用 Option 口袋

不推荐：

```rust
struct ShotTiming {
    duration: Option<Duration>,
    cue: Option<CueName>,
    voice_over: Option<AssetId>,
}
```

推荐：

```rust
enum ShotTiming {
    Fixed(Duration),
    Until(CueName),
    FromVoiceOver(AssetId),
}
```

前者允许空状态和互相冲突的组合；后者保证每个已构造值都是明确的求时规则。

### 不要用 clone 回避所有权问题

不推荐：

```rust
let encoder_frame = frame.clone();
let metrics_frame = frame.clone();
encoder.send(encoder_frame).await?;
metrics.observe(metrics_frame.len());
```

推荐先观察，再转移唯一的大 buffer：

```rust
metrics.observe(frame.len());
encoder.send(frame).await?;
```

对帧和媒体 buffer，clone 是管线决策，不是借用检查器的标点。两个消费者确实都需要所有权时，复制必须显式且可测量。

### 诊断是输出，基础设施故障才是 error

不推荐把两类问题抹平成同一个提前返回：

```rust
fn compile(source: &str) -> anyhow::Result<RenderPlan>;
```

推荐保留产品语义：

```rust
pub struct CompileReport {
    pub plan: Option<RenderPlan>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn compile(source: &str) -> Result<CompileReport, CompilerFailure>;
```

未知 cue、时长冲突属于可聚合的创作错误；素材索引不可读、内部不变式破坏才是编译器机器故障。

### Trait 必须先赢得存在理由

不推荐为每个实现套一层 Java 式抽象：

```rust
trait TimelineSolver {
    fn solve(&self, film: Film) -> Result<RenderPlan, SolveError>;
}

struct TimelineSolverImpl;
```

推荐先保留具体领域操作：

```rust
pub struct Solver {
    policy: SolvePolicy,
}

impl Solver {
    pub fn solve(&self, film: LinkedFilm) -> SolveReport {
        // ...
    }
}
```

`AssetStore`、`FrameSink`
这类实现确实会变化的边界才值得 trait。不要为稳定的内部算法套上 interface 外衣。

### 可读性高于 iterator 密度

一次干净映射适合 iterator：

```rust
let ids = shots.iter().map(Shot::id).collect::<Vec<_>>();
```

状态、诊断和顺序相互作用时，推荐有名字的 loop：

```rust
let mut cursor = TimelineTime::ZERO;

for shot in &scene.shots {
    let timing = solve_shot(shot, cursor, cues, &mut diagnostics);
    cursor = timing.end;
    solved.push(timing);
}
```

塞入
`scan`、`filter_map`、副作用和嵌套 closure 的长链，并不比明确写出时间游标更 Rust。

### 把异常路径压平

不推荐让正常路径不断缩进：

```rust
if let Some(cue) = cues.get(name) {
    if cue.at <= film.end {
        return Some(cue.at);
    } else {
        diagnostics.push(out_of_bounds(name));
    }
} else {
    diagnostics.push(unknown_cue(name));
}
```

推荐 `let ... else` 与早返回：

```rust
let Some(cue) = cues.get(name) else {
    diagnostics.push(unknown_cue(name));
    return None;
};

if cue.at > film.end {
    diagnostics.push(out_of_bounds(name));
    return None;
}

Some(cue.at)
```

正常路径应该是缩进最少的路径。

### 让可变状态只有一个所有者

不推荐把渲染器做成锁的网络：

```rust
struct Runtime {
    queue: Arc<Mutex<VecDeque<Frame>>>,
    encoder: Arc<Mutex<Encoder>>,
    progress: Arc<Mutex<Progress>>,
}
```

推荐通过有界消息转移所有权：

```rust
enum EncoderCommand {
    Frame(Frame),
    Finish(oneshot::Sender<EncodeSummary>),
}

let (commands, inbox) = mpsc::channel::<EncoderCommand>(8);
tokio::spawn(run_encoder(inbox, encoder));
```

编码任务独占 encoder。容量 `8` 属于背压设计，必须有依据、可观测。

### 让清理成为结构

不推荐会被 `?` 或取消跳过的清理：

```rust
let child = Command::new("ffmpeg").spawn()?;
render_frames().await?;
child.kill().await?;
```

推荐显式资源生命周期：

```rust
let mut encoder = EncoderProcess::spawn(config).await?;
let render_result = render_frames(&mut encoder).await;
let shutdown_result = encoder.shutdown().await;

render_result?;
shutdown_result?;
```

`Drop`
只作为强制终止的尽力保险；可能失败的异步 shutdown 必须显式保留，不能吞掉故障。

### 时间必须带单位和取整策略

不推荐：

```rust
let frame = (seconds * fps as f64) as u64;
```

推荐：

```rust
let frame = timebase.frame_at(timestamp, Rounding::Floor)?;
```

`TimelineTime`、`FrameIndex`、`FrameCount` 和 `Timebase`
即便底层都用整数，也不能互换。所有取整集中在时间 module，并在调用点命名。

## 适用范围

| 代码类型   | 职责                               | 最重要的约束       |
| ---------- | ---------------------------------- | ------------------ |
| 纯编译内核 | 解析、校验、时间求解、IR           | 全函数与诊断       |
| 计划内核   | 依赖图、缓存键、渲染计划           | 确定性与规范化数据 |
| Worker     | Chromium/FFmpeg、捕获、编码        | 资源上限与清理     |
| 编排层     | 调度、重试、组装                   | 幂等性与取消       |
| CLI        | 命令、报告、退出码                 | 稳定公开行为       |
| 浏览器桥   | 与 TypeScript runtime 的类型化消息 | 协议兼容性         |

默认禁止 `unsafe`。如未来确有已测量的需求，必须隔离在安全 API 之后，并注册例外。

## 架构

### 1. 类型就是管线

每个阶段产生不同类型，禁止不断修改一个“万能结构体”。

```rust
pub fn parse(source: &SourceDocument) -> ParseReport<ParsedFilm>;
pub fn resolve(parsed: ParsedFilm, assets: &AssetCatalog) -> ResolveReport<ResolvedFilm>;
pub fn plan(film: ResolvedFilm, profile: &RenderProfile) -> RenderPlan;
```

`ParsedFilm` 不能渲染，`ResolvedFilm` 不能包含未知 cue，`RenderPlan`
不能包含未解析素材。使用
`FrameIndex`、`FrameCount`、`CueId`、`NodeId`、`ContentHash`
等 newtype 区分底层表示相同、语义不同的值。

时间线真值不得使用无语义的 `f64`；帧率和 time base 应保留为整数或有理数。

### 2. 依赖只向内流动

纯 crate 不知道文件系统、子进程、网络、Chromium 和 FFmpeg。IO
crate 可以依赖纯 crate，反向禁止。

```text
syntax → compiler → plan
                    ↑
         worker / cli / orchestrator
```

禁止无领域归属的 `utils`、`common`、`shared` crate。一个类型属于它表达的概念。

### 3. Trait 只标记真实边界

仅在 trait 表达稳定能力边界，且存在两个真实实现、确有运行时选择、测试需替换外部边界，或它是有意设计的扩展点时引入 trait。不要对真实外部边界机械要求“必须先有两个实现”。默认静态分派，`dyn Trait`
只用于运行时选择。

### 4. 组装必须可见

长生命资源只在进程边界构造并显式传递。禁止 service
locator、可变全局注册表和隐藏 singleton。`main.rs`
应该能直接读出整个进程的组装图。

## 数据与所有权

### 5. 边界上借用，跨时间则拥有

- 仅观察时接收 `&str`、`&Path` 和 slice。
- 所有权转移时返回拥有的领域值。
- `Arc<T>` 只用于真实共享的不可变状态或已测量的跨任务需求。
- 禁止用 `clone()` 安抚借用检查器；先重新思考所有权边界。
- 公开 API 不得暴露 lock guard。

大帧缓冲和媒体 buffer 必须有一个清晰所有者。Chromium、队列和编码器之间的复制属于架构事件，必须可测量。

### 6. Enum 优于布尔盲区

使用 `CaptureMode::BeginFrame` 和 `AlphaMode::Opaque`，不使用意义不明的
`true, false`。用可辨识 enum 代替包含多个互斥 `Option`
的 struct。领域 enum 必须穷尽匹配；除非前向兼容是明确协议，否则禁止通配分支。

### 7. 转换名称必须表明是否会失败

- `From` / `Into`：无损、不会失败。
- `TryFrom` / `TryInto`：可能校验失败。
- `as_`：借用视图。
- `to_`：分配或计算新值。
- `into_`：消费 receiver。

外部文本的解析必须终止在边界；进入领域层后传递已校验类型，不继续传递字符串或
`serde_json::Value`。

## 错误与诊断

### 8. 创作错误是数据，机器故障才是 error

无效剧本是预期产品输入。返回并聚合
`Diagnostic { code, primary, message, help, related }`。字段通过只读 accessor 暴露；构造器拒绝全空白的 message、help 和 related
explanation；severity 由稳定 code 决定。在能安全报告多个错误时，不得遇到第一个未知 cue 就终止。

文件系统失败、编码器崩溃、IPC 损坏和内部不变式被破坏是执行错误。库返回类型化 error
enum，二进制边界可以增加面向人的上下文。

禁止字符串错误，禁止在库 API 中使用 `Box<dyn Error>`，禁止用 `panic!`
处理可恢复的输入或基础设施错误。

### 9. `unwrap` 必须证明不变式，否则不存在

`unwrap()` 和 `expect()` 可用于测试。生产代码中，只有在不变式已被局部证明时允许
`expect()`，且消息必须说明该不变式。`panic!`、`unreachable!` 和 `unimplemented!`
只表示程序员错误。

### 10. 错误只转换一次

在拥有第三方依赖的边界转换其错误。编译器不泄漏 XML
parser 错误，worker 不泄漏原始子进程结构；保留 source 用于调试，对外提供稳定 Onmark
code。

## 控制流与 API

### 11. 顶层函数应当读起来像编排

顶层操作应由少量命名阶段组成。多变体逻辑使用穷尽 `match` 或 handler
table。避免连续的 `if kind == ...` 同时混合识别、校验、修改和诊断。

早返回可用于消除嵌套。密集 iterator chain 并不天然比清晰的 loop 更 Rust。

### 12. 公开 API 小而可预测

- 默认 private；先选 `pub(crate)`，再考虑 `pub`。
- constructor 建立不变式。
- getter 不重复 `get_`。
- 有明确 receiver 的函数写成 method。
- 禁止 output parameter。
- 多个独立控制项使用 options struct。
- builder 只在它能消灭无效中间状态或明显改善构造时使用。
- 公开类型在语义允许时实现 `Debug`、相等、hash 和 display 等标准 trait。

Rustdoc 解释 API 为什么存在，提供真实示例，并在需要时写明
`# Errors`、`# Panics`、`# Safety`。

非平凡实现模块以简短的 inner
rustdoc 开头，说明职责、边界和主要不变式。私有状态类型应记录字段本身无法表达的信息：恢复语义、资源所有权、并发义务和协议取舍。禁止只为提高密度而复述控制流或精确的类型名称。

### 13. 协议值只有一个所有者

消息名、诊断码、文件名、环境变量、JSON 字段、缓存键部分和浏览器 global 都是协议。每个值只在所有者 crate 定义一次。线上 enum 和持久格式必须显式版本化。

会持续增长的协议 enum 使用
`#[non_exhaustive]`，要求外部消费者容忍后续新增 variant。局部校验原因 enum 在描述单个构造器的封闭失败契约时保持可穷尽；新增原因应当成为一次有意的 API 变更，而不是被 wildcard 静默吞掉。

## 确定性

### 14. 稳定输出是设计结果

- 产生字节或诊断时禁止依赖 `HashMap` 迭代顺序；必须排序或使用有序 map。
- 纯编译阶段不读取墙钟、locale、timezone、环境变量和全局随机。
- 渲染随机必须有 seed，且 seed 进入 plan hash。
- 规范化序列化只有一份实现。
- 缓存键对真正被消费的字节和所有有关环境版本做 hash。
- 等价输入产生 byte-identical IR 和稳定诊断顺序。

时间换算和 rounding 只存在于一个 module。帧边界必须具有命名的取整语义，禁止到处把秒数 cast 成整数。

### 15. 幂等性必须可观测

每个渲染任务有确定 identity。重复同一任务可以复用工作，但不得追加、重复或修改共享输出。临时结果单独写入并原子提交。

## Async、并发与子进程

### 16. 并发必须有界

- 禁止无界任务和无界 channel。
- 每个队列必须有 capacity、背压策略和所有者。
- 禁止跨 `.await` 持有 mutex guard。
- CPU 重工作不得在 async executor 上运行。
- 取消必须传递到子任务和子进程。
- 清理必须结构化且幂等。

优先使用消息传递和所有权转移，而不是 `Arc<Mutex<_>>`
网络。共享可变状态必须写明不变式。

### 17. 子进程是类型化资源

Chromium 和 FFmpeg 使用参数数组启动，永远不经过 shell 字符串。stdout/stderr 有明确大小上限。统一定义启动、就绪、优雅关闭、强制终止和进程树清理。

退出码本身不是诊断。必须转换为稳定 error，包含命令角色、有界 stderr 尾部和相关产物路径。

### 18. 背压必须传回生产者

编码慢于捕获时，降低捕获速度，不得在内存中堆积帧。buffer 数量和字节数都必须有上限且可观测。

## 性能与 unsafe

### 19. 先测量，再牺牲可读性

先优化管线边界，再优化纯函数微操作。记录分配量、复制字节、队列深度、Chromium 捕获时间、编码时间和缓存命中率。

禁止未经测量的 object pool、自定义 allocator、lock-free queue、SIMD 和
`unsafe`。

### 20. Unsafe 隔离规则

安全 crate 设置 `unsafe_code = "forbid"`，但不把它作为无差别 workspace
lint：`forbid` 无法被未来经审计的 native
bridge 降级。包含已批准 unsafe 的 crate 不继承安全 profile，改为 deny
`unsafe_op_in_unsafe_fn` 和未记录 unsafe
block。若安全代码无法满足已测量需求：隔离到专用 crate/module，只暴露安全 API，每个 unsafe
block 用 `// SAFETY:`
记录不变式，补充边界、property、Miri 和并发测试，并注册宪法例外。

## 测试

### 21. Conformance 是产品合同

合并门禁以 fixture 为中心：源文档到规范解析形式、源文档加素材到时间 IR、无效输入到稳定诊断、渲染计划到稳定字节/hash、任务到锁定环境下的确定画面/视频。

Bugfix 的第一步是制作失败回归 fixture。

### 22. 在正确层级测试

- unit：纯转换和边界。
- property：时间代数、区间关系、规范化、DAG 不变式。
- golden：诊断、IR 和 plan。
- integration：文件系统、Chromium、FFmpeg、取消和清理。
- concurrency model：只用于真正复杂的共享状态。
- benchmark：稳定代表性合成，不测玩具 loop。

尽可能通过公开 API 测试。只 mock 外部边界，不 mock 自有纯函数。

## 工具链、排版与 lint 门禁

锁定精确基线，不漂移跟随 `stable`：

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.97.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

语言 edition、MSRV 和格式 style edition 是三件事：

```toml
[workspace.package]
edition = "2024"
rust-version = "1.97"
```

```toml
# rustfmt.toml
edition = "2024"
style_edition = "2024"
max_width = 100
use_small_heuristics = "Default"
```

四空格 block indent、多行 trailing comma、100 列代码、尽量 80 列注释和 Rust 2024
version-aware sorting 交给默认 rustfmt。禁止手工对齐字段和参数。

Lint 必须按 crate 类型分层。Workspace 默认只放高信号规则；`pedantic`、`restriction`、`cargo`、`nursery`
永远不整组升为硬错误。纯编译库可禁止 panic
API，测试可使用，CLI 可通过注入的 writer 输出，库不得直接打印。

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features --keep-going
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Workspace 默认规则：Clippy `all = deny`，禁止
`dbg_macro`、`todo`、`unimplemented`；Rust 启用
`unsafe_op_in_unsafe_fn = deny`。`pedantic` 作为 crate 级 warning，不被命令行
`-D warnings` 意外提升成全组硬错误。

纯库 crate 示例：

```rust
#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr)]
```

已知单点 lint 优先使用带 `reason` 的
`#[expect(...)]`，未出现预期 lint 时会自己报告；稳定的 crate 级分歧才使用带文档的
`allow`。Onmark 策略例外仍使用：

```rust
// onmark-exception: R<clause> <one-sentence reason>
```

## 一票否决的反模式

- 时间线秒数使用无语义浮点数；
- 领域 ID 互相兼容的字符串；
- 一个可变 struct 贯穿全部编译阶段；
- 用 `pub` 代替包设计；
- 无领域主人的 `utils/common/shared`；
- 只有一个实现且不代表边界的 trait；
- 用 clone 消灭所有权错误；
- 把 `Arc<Mutex<_>>` 当作默认架构；
- 跨 `.await` 持有 lock；
- 无界 channel 或任务；
- 用 shell 字符串启动 Chromium/FFmpeg；
- `serde_json::Value` 越过外部解析边界；
- 持久输出依赖无序 map 迭代；
- 配置边界之外直接读环境变量；
- 用 `unwrap`、`panic!` 或首错即停处理创作输入；
- 没有代表性 benchmark 的优化；
- 注释复述语法，而不解释不变式和取舍。

## 来源

本规范于 2026-07-11 按
[Rust 1.97.0](https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/)、官方
[Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)、[Rust 2024 Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/)、[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)、[rustc 开发规约](https://rustc-dev-guide.rust-lang.org/conventions.html)、官方
[Clippy](https://doc.rust-lang.org/stable/clippy/) 以及 Google 持续维护的
[Idiomatic Rust](https://google.github.io/comprehensive-rust/idiomatic/welcome.html)
完成审计。确定性时间、媒体 buffer、子进程和渲染计划规则是 Onmark 的领域约束，不是通用 Rust 律法。
