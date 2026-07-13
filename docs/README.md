# Onmark engineering documents

English and Chinese documents share the same decisions. Chinese architecture documents currently carry the fuller design discussion; English documents provide the maintained project-facing specification.

| Document | Purpose |
| --- | --- |
| [`en/rust-style-guide.md`](en/rust-style-guide.md) | The code constitution for Rust crates |
| [`zh-CN/rust-style-guide.md`](zh-CN/rust-style-guide.md) | Chinese mirror |
| [`en/typescript-style-guide.md`](en/typescript-style-guide.md) | The code constitution for TypeScript and JavaScript |
| [`zh-CN/typescript-style-guide.md`](zh-CN/typescript-style-guide.md) | 中文 TypeScript/JavaScript 代码宪法 |
| [`en/presentation-contract.md`](en/presentation-contract.md) | Browser presentation authoring contract |
| [`zh-CN/presentation-contract.md`](zh-CN/presentation-contract.md) | 浏览器 presentation authoring 合约 |
| [`en/architecture.md`](en/architecture.md) | Target architecture and execution model |
| [`zh-CN/architecture.md`](zh-CN/architecture.md) | 中文架构设计与完整渲染链路 |
| [`en/language-specification.md`](en/language-specification.md) | Screenplay language semantics and diagnostics |
| [`zh-CN/language-specification.md`](zh-CN/language-specification.md) | 剧本语言语义、求时规则与诊断规范 |

Current audited baseline: Rust 1.97.0, language edition 2024, style edition 2024; TypeScript 7.0.2, Node.js 26.4.0, pnpm 11.9.0 (2026-07-11).

The style guides separate mechanical formatting, language idiom, and Onmark-specific engineering law. The constitutions govern call-site clarity, ownership, state, errors, determinism, concurrency, process boundaries, and tests.
