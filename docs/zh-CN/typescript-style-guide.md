# Onmark TypeScript 代码宪法

> 基线：TypeScript 7.0.2、Node.js 26.4.0、pnpm 11.9.0。English:
> [English](../en/typescript-style-guide.md)

漂亮的 TypeScript 应让所有权、protocol
state、异步边界与浏览器副作用直接显现在类型和控制流中。Onmark 用 TypeScript 承担 authoring、bundling
与 browser runtime，但绝不在 TypeScript 里重做 Rust 已经求出的时间或规划事实。

## 适用范围

| 类型              | 例子                          | 首要风险                   |
| ----------------- | ----------------------------- | -------------------------- |
| Browser runtime   | clock、session、DOM adapter    | 确定性状态与有界 readiness |
| Node toolchain    | authoring、bundler、generator | 显式 IO 与可复现输出       |
| Wire boundary     | 生成类型与 codec              | 单一事实源与兼容性         |
| Tests/conformance | fake、fixture、smoke          | 公开行为而非内部实现       |

生成文件服从 generator，禁止手改。评审生成代码时，应检查 Rust wire
type、schema、generator 与 drift gate，而不是逐行润色第三方机械输出。

## 架构

### 1. Rust 拥有时间事实，TypeScript 拥有浏览器副作用

创作语义、求时、区间、分片与 Execution
Plan 只由 Rust 拥有。TypeScript 消费 versioned
facts，并把它们施加到 DOM、CSS、Canvas、WebGL 与 browser media
API。TypeScript 可以校验 wire
contract，也可以从精确帧事实推导浏览器 API 参数；不得出现第二套 timing
solver、cue resolver 或 partition policy。

### 2. 一个概念只有一个事实源

- Rust wire types 生成 checked-in JSON Schema 与 TypeScript types/codecs；
- protocol union、code、version、field name 不得手写复制；
- 跨文件的 protocol string 在 owner package 里只定义一次；
- 提交进仓库的生成物必须有确定性的 `--check` CI gate；
- 复制常量默认是缺陷，除非依赖边界禁止共享且有测试钉住一致性。

### 3. 依赖通过 package facade 单向流动

package 只能从另一个 package 的公开 `exports` 导入，不能 reach into `src/`。
`@onmark/runtime` 永不依赖 authoring 或 bundler。生产 browser 模块不得导入 Node built-in。runtime 内部可以消费生成代码，外部消费者
只看 `src/index.ts` 的公开面。

禁止
`utils.ts`、`helpers.ts`、`shared.ts`、`common.ts`。函数应和它所保护的领域不变量放在一起。

### 4. 副作用只从窄能力边界进入

纯 recognition、validation、state
transition 与 formatting 不接触 DOM、filesystem、environment、network 或 subprocess。浏览器与 Node 副作用通过显式传入的窄接口进入；长寿命资源在 process/session 边界构造。禁止 service
locator、可变全局 registry 与 decorator-based injection。

一个真实外部实现加一个测试 fake，足以证明接口存在价值；稳定内部算法不需要披上 interface 外套。

## 类型与所有权

### 5. 让非法状态难以表达

protocol phase 与 result alternative 使用 discriminated
union，避免用一袋 optional field 表示互斥状态。parse boundary 使用 `unknown`
并立即 narrow，禁止 `any`。封闭 protocol variant 优先 exhaustive `switch`。

借用事实与公开 view 使用 `readonly`。可变输入一旦跨越异步生命周期，必须在第一个
`await` 前取得 owned snapshot；不得保存调用方数组或对象并假设它以后不变。

### 6. class 必须真正拥有身份或生命周期

无状态变换默认使用函数。class 只在以下情况成立：

- `Error` subtype；
- 拥有可变 protocol state 的 session/browser resource；
- `HTMLElement` 等平台强制类型。

禁止把依赖装进
`XxxService`、`XxxManager`、`XxxRepository`。平台 class 只做薄 glue：读取浏览器状态、调用聚焦操作、应用结果。

### 7. 名字暴露单位与选择

调用点应能直接读出英文语义。文件名是概念，函数名是动作，类型名是名词，错误以
`Error` 结尾。frame、seconds、hash、request ID、interval 不得退化成可互换的
`value`。

独立控制项用 options object，互斥选择用 discriminated union 或 enum-like string
union；避免 boolean blindness。

## 控制流与失败

### 8. 控制流保持块状

顶层编排应像目录。用 exhaustive `switch`
显示主要 variant 轴，再把实质性 variant 放进矩形操作。优先 guard clause 与
`if ... else` narrowing，避免嵌套金字塔和带副作用的密集 iterator chain。

不要为了缩短行数提取 helper。只有当代码块拥有稳定领域名、保护不变量或隔离机械边界时才提取。

### 9. 预期失败是数据，意外失败才 throw

非法创作输入与 protocol rejection 是正常产品输出，应返回 diagnostic 或 typed
failure
event；安全时聚合独立错误。基础设施错误、不可能状态与内部前置条件破坏才 throw。

`try/catch` 只允许出现在能真正翻译语义的位置：protocol/process
boundary、第三方异常转 Onmark typed failure、resource cleanup、concurrent
aggregation。禁止 catch 后只改名字或静默继续。未类型化 adapter
exception 在 RuntimeSession 边界收敛，不得泄露 vendor-dependent
message 到 protocol。

### 10. 异步工作与清理必须有界

每个 wait 都有 owner；依赖外部状态的 wait 必须有 deadline。每条 queue 必须有容量，否则直接拒绝并发。禁止在友好 API 后藏无界 promise
chain。未知 browser component 默认 sequential，随机 seekability 必须证明。

cleanup 显式且 terminal。dispose 失败仍可观测，但半清理 session 不得重新服务。fire-and-forget
promise 必须有结构化 owner 并显式标注；意外 floating promise 是缺陷。

### 11. 确定性是类型级问题

浏览器输出只由 frame index 与 rational timebase 驱动，禁止 wall
time、`Date.now()`、ambient animation progress 与未 seeded
randomness 决定捕获帧。影响 wire
output、hash、diagnostic 或生成字节的迭代必须稳定排序。hash 应覆盖浏览器实际消费的字节，而不是看起来相近的旁边源码。

## 文件、注释与公开面

### 12. 文件自带导航

每个手写 TS/JS 文件顶部都用短 header 说明“它拥有什么、为何存在”。超过 200 行的文件用 section
divider 标出主要概念块。生成文件只需带 source-of-truth banner。

模块形成树而不是碎纸屑。同一原因变化的代码保持在一起；只有 section
owner 不同时才拆文件，不为行数指标拆分。

### 13. 注释解释约束，不复述语法

跨异步边界的所有权、并发竞态、cleanup 决策、反直觉浏览器行为与 protocol 取舍必须注释。禁止给循环配旁白、重复变量名、保存历史或写裸
`TODO`；延后项必须说明 owner 与触发条件。

公开入口只 re-export 有意设计的窄表面；测试和其他 package 不能 reach
into 内部文件。

## 测试与生成代码

### 14. 测试通过公开 API 断言行为

纯函数直接调用，只 fake browser
adapter、filesystem、process 等外部能力；禁止用 mock framework
patch 内部函数。package behavior test 放在
`test/`，概念名与源码对应；跨语言行为放在根 `conformance/`。

bugfix 第一步是失败的 focused test/fixture。snapshot-style
golden 只属于 conformance 和生成物，不能代替行为断言。

### 15. script 也是生产级构建代码

generator 与 CI script 使用 named constant、稳定顺序、显式 exit
status、可行动 stderr 和只读 check
mode。它们遵守与 package 相同的 header、命名、失败与格式规则；check 命令不得修改仓库。

## 工具基线

`tsc --noEmit` 必须启用 `strict`、`noUncheckedIndexedAccess`、
`exactOptionalPropertyTypes`、`noImplicitOverride`、
`noPropertyAccessFromIndexSignature`、`isolatedModules` 与
`verbatimModuleSyntax`。lint 禁止显式 `any`、产品代码 default
export、`console`、不一致 type import 与直接读取
`process.env`。格式完全机械化并由 CI 检查。根目录 `conformance/`
下的手写 browser source 必须和 package source 一样经过 strict
typecheck、lint、shape 与 format gate；bundler
build 成功不能替代类型检查。生成输出不参与手工 formatter，而由 regeneration
gate 管理。

## 一票否决反模式

- TypeScript 重做 Rust timing、cue 或 partition logic；
- 手写复制生成 protocol type/code；
- generated third-party output 之外出现 `any`；
- 无界 queue、wait、retained buffer 或 promise chain；
- free-running browser time 决定捕获帧；
- 无 discriminant 的万能可变 runtime object；
- 跨 `await` 保存 caller-owned mutable data；
- service locator、DI container 或 dependency-bag class；
- `utils/helpers/shared/common` 垃圾场；
- 产品模块 default export、`console.log`、ambient `process.env`；
- 手改生成文件或会写仓库的 drift check；
- 把下一行翻成英文的注释。

## 来源与差异

本规范在 2026-07-11 审阅 uiku 的 TypeScript 代码宪法、runtime、toolchain、测试、lint 配置与 style
drift
check 后，为 Onmark 重新设计。两者不机械同步：Onmark 的视频求时只归 Rust，`RuntimeSession`
是合法 lifecycle class，Node 原生 test
runner 已足够，browser/runtime 依赖预算服从 Onmark 自己的 delivery gate。
