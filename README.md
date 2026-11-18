# YunYou

A terminal AI Agent written in Rust: component kernel + fully pluggable motherboard.
Conversation, tool calls, session persistence, multi-process team collaboration, sandboxing —
every capability is a card plugged into a kernel slot.

> Provenance: a Rust rewrite inspired by the architecture of DeepSeek Harness (dsh).
> Internal development happens in a private repository; this GitHub repo is the open-source release surface.

## One-command install (global)

```bash
curl -fsSL https://github.com/5201314155/yunyou/raw/main/install.sh | sh
```

Installer behavior: downloads the prebuilt binary from GitHub Releases (**no Rust toolchain
required**) → falls back to `cargo install --git` (source build) → fixes PATH automatically.
Upgrading = re-run the same command. Uninstall = `rm ~/.local/bin/yunyou`.

**官方插件随装清单（v4，随主程序同通道）**：

| 插件 | 落位 | 模型面 |
| --- | --- | --- |
| scheduler | `~/.yunyou/plugins/workflows/scheduler/` | schedule_create / schedule_list / schedule_delete（定时任务） |
| web-search | `~/.yunyou/plugins/tools/web-search/` | web_search（联网搜索，DDG 零配置默认；SearXNG/Brave/Tavily 可配切换） |

单件下载失败自动跳过不阻塞主程序；memory 插件候入库后加行（设计已批，代码未落）。

## Architecture (motherboard / slots / data flow)

The kernel is a motherboard: it only provides slot specs (12 frozen contracts + UI registration
seams). Everything else — LLM backends, tools, sessions, prompts, retries, sandboxing, teams,
scheduled triggers — is a plugin card. Changing a spec means shipping a new slot version;
silent changes are forbidden.

- `crates/` — kernel & bundled components, grouped by domain (core/llm/session/tools/shell/team/net/infra/plugins)
- `sdk/` — `yunyou-plugin-sdk`: the only dependency a plugin author needs (compiles standalone, zero kernel deps, CI-asserted)
- `plugins/<category>/<name>/` — runtime install surface (auto-classified on install)
- `frontend/` — web frontend (separate from the Rust tree)

## Capability overview (all plugin-shaped, visible under `/plugins`)

- **Tools**: six built-ins + external SDK plugins
- **Persona**: `AGENTS.md` four-tier chain — `/etc/yunyou/AGENTS.md` (managed) →
  `~/.yunyou/AGENTS.md` (user) → `<project>/AGENTS.md` → `<project>/.yunyou/local/AGENTS.md`
  (local, not committed). Injected in order, later layer wins; duplicate rules across
  layers get an explicit conflict note (better than claude's silent override); each
  layer is anchored `[agents:<tier>]` for future hook policies. Legacy fallback:
  `.yunyou/persona.md` > `~/.yunyou/persona.md` > toml string — all take effect on
  next start, zero code
- **Skills**: `.yunyou/skills/*.md` auto-registered into a model-facing catalog, read on demand (progressive disclosure)
- **Permissions**: four modes (default / acceptEdits / plan / bypass) + workspace boundary + always-on dangerous-command interception
- **Scheduling**: the model picks its own times via `schedule_create` (cron / interval / one-shot)
- **Modes**: `/mode plan|work|creation` (plan-then-approve / default / creation mode — with creation mode off, the model has no plugin-development capability)
- **Plugin management**: `yunyou plugin new|check|install|outdated|update|remove` (three update sources + contract compatibility gate)

## Writing a plugin (five minutes)
## Installing plugins (one command)

```bash
yunyou plugin install scheduler        # official plugin: pull manifest+arch-matched binary from release-bin
yunyou plugin install <git url>        # community plugin: shallow clone, install from repo
yunyou plugin install ./my-plugin      # local directory (manifest.toml + built binary)
```

All three paths run the same validation chain (manifest five-class checks + duplicate guard) and land under `<home>/plugins/<category>/<id>/`. Restart yunyou → `/plugins` shows it → model can call its tools.


```bash
yunyou plugin new weather --category tools   # scaffolds manifest + template main + README
# edit src/main.rs — argument schemas are derived and surfaced to the model automatically
cargo build                                  # compiles the plugin only, never the kernel
yunyou plugin install ./weather              # auto-classified into plugins/tools/weather/
# restart yunyou → /plugins shows it, switchable → the model can call your tool
```

Plugin authors depend only on `yunyou-plugin-sdk` (git/registry dependency — never a path into
the kernel tree). See `sdk/yunyou-plugin-sdk/SDK.md` for the capability matrix
(what each of the 12 seams lets you build, with minimal examples and permission boundaries).

## Configuration (yunyou.toml)

TOML, every section optional, unknown keys fail loudly (no silent typos). Full annotated
example in [config.example.toml](config.example.toml) (minimal: [config.minimal.toml](config.minimal.toml); 中文注解版见 [README.zh.md](README.zh.md)). Highlights:

```toml
[llm]
base_url = "https://api.example.com/v1"
api_key = "$YUNYOU_LLM_API_KEY"        # credential reference — resolved from the credential store
model = "vendor/model-name"

[agent_loop]
tool_timeout_secs = 300
# max_steps / token_budget: optional guardrails (soft landing, resumable)

[sandbox]
enabled = true                          # Landlock + seccomp (Linux)
network_denied = true
```

## Credentials

Recommended: store the real key in `~/.yunyou/credentials.toml` (0600, auto-managed) and
reference it from the config as `$YUNYOU_LLM_API_KEY`. Keys never live in configs, logs,
or code. Env fallback: `DSH_TEST_LLM_BASE_URL` / `DSH_TEST_LLM_API_KEY` / `DSH_TEST_LLM_MODEL`.

## Slash commands

`/help` `/new` `/sessions` `/plugins` `/quit` — plus anything registered by plugins
(skills are auto-registered as commands too). Non-interactive: `yunyou "summarize this repo"`.

## Sandbox (Linux)

Bash subprocesses run under Landlock (fs allowlist) + seccomp (network denial):

- writes confined to the workspace root and /tmp
- `network_denied = true` blocks subprocess networking (real failures surface to the model)
- unsupported architectures fail closed — never silently unsandboxed

## Session semantics (claude-parity)

Every launch starts a **new session** (multiple instances never collide). Resume with
`yunyou --resume` (interactive picker) or continue with `yunyou --continue`;
`--fork` copies a locked session into a new one.

## Self-update

```bash
yunyou update        # downloads the latest prebuilt binary, atomic replace, .bak rollback
yunyou update --check
```

## Development

```bash
cargo build --workspace
cargo test --workspace          # includes the tests/ integration targets
cargo clippy --workspace --all-targets
```

See `docs/接口/` for machine-readable plugin contracts (tools/commands/manifest/sandbox/message v1)
and `sdk/yunyou-plugin-sdk/SDK.md` for the capability matrix.

## License

Apache-2.0 — see THIRD_PARTY_NOTICES.md for third-party license register.
