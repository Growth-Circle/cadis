# Business Requirements Document

## 1. Business Context

CADIS is an open-source local AI runtime project. The business objective is not to build another hosted chatbot. The objective is to create a trusted, extensible, fast local runtime that can become the foundation for tools, agents, desktop workflows, and optional future commercial services.

The immediate reason for the reset is performance and control. A core built on OpenClaw is considered too slow and too constrained for the desired agent operating layer. A clean Rust-first foundation creates better long-term leverage.

## 2. Business Objectives

| ID | Objective | Measure |
| --- | --- | --- |
| BRD-001 | Establish CADIS as a credible open-source runtime | Public repo has complete docs, license, contribution rules, roadmap |
| BRD-002 | Build a fast local daemon foundation | First event latency and tool dispatch targets are measured |
| BRD-003 | Reduce provider lock-in | At least two providers pass conformance tests by alpha |
| BRD-004 | Enable real coding workflows | Worktree-isolated coding flow reaches patch review |
| BRD-005 | Build trust through safety | Approval, logging, and redaction behavior are documented and tested |
| BRD-006 | Enable remote control | Telegram adapter supports command, status, and approval flow |

## 3. Value Proposition

CADIS gives technical users a local command center for AI work:

- One daemon coordinates all interfaces.
- Agents can run without freezing the main conversation.
- Dangerous actions are not hidden inside chat.
- Code-heavy work is visible and reviewable.
- The user can switch models without replacing the runtime.
- The system can be extended without turning every integration into core logic.

## 4. Target Market

### Initial Market

- Developers and operators who use local Linux machines.
- AI power users who already run CLI tools and local models.
- Builders who want Telegram or desktop control over local agents.

### Later Market

- Teams that need local-first AI task execution.
- Open-source contributors building model providers and tool adapters.
- Organizations that need private agent orchestration with explicit approvals.

## 5. Distribution Model

CADIS starts as open-source software.

Possible future distribution paths:

- GitHub releases with static binaries.
- Linux packages.
- Homebrew for macOS later.
- Windows installer later.
- Optional hosted relay or team features later.

The core daemon should remain useful without paid infrastructure.

## 6. Monetization Options

No monetization is required for the first release. Future options must not compromise local-first use:

- Paid hosted sync or remote relay.
- Managed team policy templates.
- Enterprise support.
- Private plugin registry.
- Hosted observability dashboard.
- Commercial license for bundled enterprise components if needed.

## 7. Business Constraints

- The project must remain publishable as open source.
- Third-party source code cannot be imported without license review.
- Core dependencies should be small, maintained, and compatible with Apache-2.0 distribution.
- Model provider terms may restrict usage patterns; provider integrations must be modular.
- Telegram and TTS integrations must remain optional.
- Security issues can damage trust quickly because the product executes tools.

## 8. Stakeholder Requirements

| Stakeholder | Requirement |
| --- | --- |
| Maintainer | Clear architecture and low dependency sprawl |
| Contributor | Easy onboarding and scoped issues |
| User | Fast, safe, useful local runtime |
| Security reviewer | Documented threat model and central policy |
| Future sponsor | Visible roadmap and credible governance |

## 9. Operational Requirements

- Releases should use semantic versioning after first tagged release.
- Changelog must be maintained.
- Security policy must be visible.
- CI must check repository hygiene first, then build/test as code is added.
- Every core crate should have owner, purpose, and public API boundary.
- Public docs must avoid secrets, private machine paths, and provider keys.

## 10. Compliance and License Requirements

- Baseline license: Apache-2.0.
- Any imported upstream code must keep required license headers and notices.
- If Codex-derived code is used, the decision must be recorded before import.
- Dependencies must be tracked before release.
- Generated code must be clearly identified if committed.

## 11. Business Success Metrics

- First public release published without license blockers.
- Setup path documented in under 10 minutes for Linux user.
- At least one real coding workflow completed end-to-end.
- At least one remote approval flow completed through Telegram.
- At least one external contributor can understand architecture from docs alone.

## 12. Assumptions

- Rust is acceptable for the entire core.
- Linux desktop is the right first platform.
- Users value local control more than browser-first convenience.
- A daemon-first architecture will outperform UI-first assistant shells.
- Open-source credibility requires docs and governance before hype.

## 13. Business Risks

- Scope expands into an IDE, SaaS, model platform, and automation suite too early.
- UI work starts before daemon behavior is stable.
- Too many providers are built before one provider contract is proven.
- Security policy is added late and becomes inconsistent.
- Public release includes unclear upstream licensing.

## 14. Exit Criteria for Planning Phase

- Product, business, functional, technical, architecture, roadmap, checklist, risk, and decision docs exist.
- Open-source files exist.
- The first implementation sprint is explicit enough to start without redesign.

