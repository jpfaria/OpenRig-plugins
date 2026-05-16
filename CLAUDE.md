# OpenRig-plugins — Claude Code

Repositório de **binários de plugins pré-compilados** (LV2 nativos + capturas NAM/IR) consumidos pelo [OpenRig](https://github.com/jpfaria/OpenRig). Cada plugin vive em `plugins/source/<kind>/<name>/` com `manifest.yaml`, `data/` (TTL) e `platform/<slot>/<lib>`.

## Invariante crítico — slot é single source of truth do OpenRig

Os nomes de slot de plataforma em `manifest.yaml` (`binaries:`) e na toolchain (`scripts/build-lib.sh`, `.github/workflows/build-libs.yml`) **TÊM que bater exatamente** com o enum `Lv2Slot` do OpenRig (`crates/plugin-loader/src/manifest.rs`):

```
macos-universal · windows-x86_64 · windows-aarch64 · linux-x86_64 · linux-aarch64
```

**NUNCA** inventar/renomear slot aqui (ex.: `windows-x64`, `windows-arm64`). O enum é a fonte; serde alias no OpenRig é proibido. Slot divergente = `pack_plugins` falha → release do OpenRig quebra inteira (issue #5). Mudou plataforma? Alinha ao enum do OpenRig **primeiro**.

## Gate obrigatório antes de qualquer push

```
cargo run --release --bin pack_plugins
```

Exit 0 / `0 failed`. É o MESMO gate do job `Bundle plugins` do `release.yml` do OpenRig. Vermelho aqui = release vermelha lá.

## Gitflow

Mesmo processo do OpenRig: **issue → `bugfix/issue-N` (ou `feature/issue-N`) a partir de `main` → commits em inglês (sem `Co-Authored-By`, sem `Fixes #`) → PR pra `main`**. Bugfix mergeia imediato após review; PR/merge só com pedido explícito do usuário. Workspace isolado em `.solvers/issue-N/` (nunca editar no working dir do usuário). Comentar na issue: plano antes de começar, cada push (hash + arquivos + gate), resumo final.

## Metodologia de código

Ver `.claude/skills/openrig-code-quality/SKILL.md` — regras de qualidade language-agnostic (zero coupling, single source of truth, separação de concerns, organização de arquivo, TDD, docs em sincronia, comunicação objetiva). Invocar antes de qualquer ação não-trivial.
